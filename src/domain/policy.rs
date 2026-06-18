//! Canonical policy hashing (artifact-binding's sibling for *rules*).
//!
//! `policy_hash` answers: "were these gate/audit results produced under the same
//! task policy that is in force now?" It binds the evidence to *what was allowed
//! and what had to be verified*, not to the code (that is `tree_hash`).
//!
//! The hash is derived from a canonical byte encoding of the policy STRUCTURE —
//! never from an arbitrary serialization — so two semantically identical policies
//! (same sets, any field/order) always produce the same hash. Only fields that
//! change "what is allowed / what must pass" are included; cosmetic fields
//! (objective wording, task_id, timestamps, actor, phase, history, gate *results*)
//! are deliberately excluded so unrelated edits never invalidate evidence.

use sha2::{Digest, Sha256};

/// Bumped when the canonical encoding changes, so old and new hashes never
/// collide across an encoding revision.
const POLICY_SCHEMA_VERSION: u32 = 1;

/// Version of the **executor governance policy** — the rules under which a run
/// executes and its output is admitted: the run-manifest contract
/// (`control.run-manifest.v1`), the ingest scope check (SCOPE-001), and
/// `validate_output` (evidence `source` + `touched_files`). It is folded into
/// every `policy_hash` so that gate/audit evidence is bound to the executor
/// policy generation in force when it was produced: bump this and all prior
/// evidence goes stale (its `policy_hash` no longer matches), forcing re-run
/// under the new policy. It is deliberately separate from
/// [`POLICY_SCHEMA_VERSION`] (which only tracks the byte *encoding*).
pub const EXECUTOR_POLICY_VERSION: u32 = 1;

/// The policy-relevant definition of a single required gate. Carries the gate's
/// *actual* command + args, not just its id — renaming `cargo check` to
/// `cargo check --all-targets` behind the same id must change the hash.
#[derive(Debug, Clone)]
pub struct CanonicalGateDefinition {
    pub gate_id: String,
    pub command: String,
    pub args: Vec<String>,
}

/// Length-prefixed string write: `len(u32 LE) || bytes`. Length-prefixing makes
/// the encoding unambiguous regardless of path contents (no separator can be
/// forged by embedding it in a value).
fn push_str(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
    buf.extend_from_slice(s.as_bytes());
}

/// Write a labeled set field: items are sorted + deduped so order/duplication
/// never affects the hash (set semantics).
fn push_set(buf: &mut Vec<u8>, label: &str, items: &[String]) {
    push_str(buf, label);
    let mut sorted: Vec<&String> = items.iter().collect();
    sorted.sort();
    sorted.dedup();
    buf.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    for it in sorted {
        push_str(buf, it);
    }
}

/// Compute the canonical `policy_hash` for a task policy.
///
/// `required_gates` may be passed in any order; gate defs are sorted by `gate_id`.
/// `args` order IS significant (it is the command line) and is preserved.
pub fn compute_policy_hash(
    read_scope: &[String],
    write_allow: &[String],
    write_deny: &[String],
    risk_triggers: &[String],
    required_gates: &[CanonicalGateDefinition],
) -> String {
    // Bind the current executor policy generation into every hash. Kept as an
    // inner fn taking the version explicitly so the binding is unit-testable
    // without a runtime-mutable const, while the public signature is unchanged.
    compute_policy_hash_inner(
        read_scope,
        write_allow,
        write_deny,
        risk_triggers,
        required_gates,
        EXECUTOR_POLICY_VERSION,
    )
}

fn compute_policy_hash_inner(
    read_scope: &[String],
    write_allow: &[String],
    write_deny: &[String],
    risk_triggers: &[String],
    required_gates: &[CanonicalGateDefinition],
    executor_policy_version: u32,
) -> String {
    let mut buf = Vec::new();
    buf.extend_from_slice(&POLICY_SCHEMA_VERSION.to_le_bytes());

    // Executor governance policy generation (see EXECUTOR_POLICY_VERSION).
    push_str(&mut buf, "executor_policy_version");
    buf.extend_from_slice(&executor_policy_version.to_le_bytes());

    push_set(&mut buf, "read_scope", read_scope);
    push_set(&mut buf, "write_allow", write_allow);
    push_set(&mut buf, "write_deny", write_deny);
    push_set(&mut buf, "risk_triggers", risk_triggers);

    // Required gates: sort by id (set over gates), but keep each gate's args order
    // (the command line is ordered).
    let mut gates: Vec<&CanonicalGateDefinition> = required_gates.iter().collect();
    gates.sort_by(|a, b| a.gate_id.cmp(&b.gate_id));
    gates.dedup_by(|a, b| a.gate_id == b.gate_id);
    push_str(&mut buf, "required_gates");
    buf.extend_from_slice(&(gates.len() as u32).to_le_bytes());
    for g in gates {
        push_str(&mut buf, &g.gate_id);
        push_str(&mut buf, &g.command);
        buf.extend_from_slice(&(g.args.len() as u32).to_le_bytes());
        for a in &g.args {
            push_str(&mut buf, a);
        }
    }

    format!("{:x}", Sha256::digest(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(id: &str, cmd: &str, args: &[&str]) -> CanonicalGateDefinition {
        CanonicalGateDefinition {
            gate_id: id.to_string(),
            command: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn identical_policy_same_hash() {
        let g = vec![gate("cargo_check", "cargo", &["check"])];
        let a = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &g);
        let b = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &g);
        assert_eq!(a, b);
    }

    /// THE key test: different field/set order, same semantics → same hash.
    /// This is what proves it hashes the policy, not the serialized text.
    #[test]
    fn set_order_does_not_change_hash() {
        let g1 = vec![
            gate("cargo_check", "cargo", &["check"]),
            gate("cargo_test", "cargo", &["test"]),
        ];
        let g2 = vec![
            gate("cargo_test", "cargo", &["test"]),
            gate("cargo_check", "cargo", &["check"]),
        ];
        let a = compute_policy_hash(
            &s(&["a", "b"]),
            &s(&["x", "y"]),
            &[],
            &s(&["r1", "r2"]),
            &g1,
        );
        let b = compute_policy_hash(
            &s(&["b", "a"]),
            &s(&["y", "x"]),
            &[],
            &s(&["r2", "r1"]),
            &g2,
        );
        assert_eq!(a, b, "set/gate ordering must not affect policy hash");
    }

    #[test]
    fn widening_write_allow_changes_hash() {
        let g = vec![gate("cargo_check", "cargo", &["check"])];
        let narrow = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &g);
        let wide = compute_policy_hash(&s(&["src"]), &s(&["src", "schemas"]), &[], &[], &g);
        assert_ne!(narrow, wide);
    }

    #[test]
    fn narrowing_write_allow_changes_hash() {
        let g = vec![gate("cargo_check", "cargo", &["check"])];
        let wide = compute_policy_hash(&s(&["src"]), &s(&["src", "tests"]), &[], &[], &g);
        let narrow = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &g);
        assert_ne!(wide, narrow);
    }

    #[test]
    fn gate_args_change_changes_hash() {
        let before = vec![gate("cargo_check", "cargo", &["check"])];
        let after = vec![gate("cargo_check", "cargo", &["check", "--all-targets"])];
        let a = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &before);
        let b = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &after);
        assert_ne!(a, b, "changing a gate's command line must change the hash");
    }

    #[test]
    fn required_gate_set_change_changes_hash() {
        let one = vec![gate("cargo_check", "cargo", &["check"])];
        let two = vec![
            gate("cargo_check", "cargo", &["check"]),
            gate("cargo_test", "cargo", &["test"]),
        ];
        let a = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &one);
        let b = compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &two);
        assert_ne!(a, b);
    }

    /// gate_id collision with different command: encoding must not let
    /// "cargo_check"+"cargo" be confused with "cargo_chec"+"kcargo".
    #[test]
    fn length_prefixing_prevents_field_collision() {
        let a = compute_policy_hash(&s(&["ab"]), &[], &[], &[], &[]);
        let b = compute_policy_hash(&s(&["a", "b"]), &[], &[], &[], &[]);
        assert_ne!(a, b);
    }

    /// Bumping the executor policy generation must change the hash, so all prior
    /// gate/audit evidence goes stale and must be re-produced under the new
    /// executor policy.
    #[test]
    fn executor_policy_version_change_changes_hash() {
        let g = vec![gate("cargo_check", "cargo", &["check"])];
        let v1 = compute_policy_hash_inner(&s(&["src"]), &s(&["src"]), &[], &[], &g, 1);
        let v2 = compute_policy_hash_inner(&s(&["src"]), &s(&["src"]), &[], &[], &g, 2);
        assert_ne!(
            v1, v2,
            "bumping the executor policy version must change the policy hash"
        );
    }

    /// The public hash binds the current `EXECUTOR_POLICY_VERSION`.
    #[test]
    fn public_hash_binds_executor_policy_version_constant() {
        let g = vec![gate("cargo_check", "cargo", &["check"])];
        assert_eq!(
            compute_policy_hash(&s(&["src"]), &s(&["src"]), &[], &[], &g),
            compute_policy_hash_inner(
                &s(&["src"]),
                &s(&["src"]),
                &[],
                &[],
                &g,
                EXECUTOR_POLICY_VERSION
            ),
            "public compute_policy_hash must fold in EXECUTOR_POLICY_VERSION"
        );
    }
}
