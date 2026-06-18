#!/usr/bin/env bash
# adapter_parity_e2e.sh — adapter-same-task-e2e-v1
#
# Deterministic, model-free harness that runs the *identical* ctl task lifecycle
# through every supported executor adapter (omp, opencode) and captures the
# governance behavior so it can be compared side by side. See the report at
# ADAPTER_PARITY_E2E.md for the dimension-by-dimension result and disclosures.
#
# Per adapter, in isolated throwaway git+cargo repos:
#   HAPPY task  — create → ready → start → run start → in-scope ingest → submit
#                 → commit → gate → audit → finish → archive. Yields: lifecycle,
#                 evidence source, gate binding, audit, finish/archive, and
#                 `ctl adapter doctor --json` before vs after.
#   DENY  probe — a SEPARATE governed task whose run ingests an out-of-scope
#                 `touched_files` → SCOPE-001 `evidence_rejected`, plus the live
#                 `ctl hook gate` on the same path → allowed:false. Isolated
#                 because the completion interlock (STATE-012) deliberately
#                 blocks finish while a rejection is unresolved.
#
# Determinism: NO model is invoked anywhere. The "agent output" is a pre-written
# agent-output.json; the DENY is produced by declaring an out-of-scope touched
# file, not by a model attempting an out-of-scope edit. See ADAPTER_PARITY_E2E.md
# "Disclosure: deterministic vs model-driven deny".
#
# Usage:  scripts/adapter_parity_e2e.sh [OUT_DIR]
#   CTL=/path/to/ctl  scripts/adapter_parity_e2e.sh
# Exit 0 iff governance is identical across adapters on every shared dimension
# (the evidence `source` tag is expected to differ — that is the only divergence).

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CTL="${CTL:-$REPO_ROOT/target/debug/ctl}"
ADAPTERS=("omp" "opencode")
OUT="${1:-$(mktemp -d)}"
mkdir -p "$OUT"

if [[ ! -x "$CTL" ]]; then
  echo "FATAL: ctl binary not found/executable at: $CTL (build it: cargo build)" >&2
  exit 2
fi

EVENT_KEY=""
# Ordered list of event variant names from a ledger file (key auto-detected).
event_kinds() {
  local f="$1"
  if [[ -z "$EVENT_KEY" ]]; then
    for k in event_type type kind event; do
      if grep -q "\"$k\"[[:space:]]*:" "$f" 2>/dev/null; then EVENT_KEY="$k"; break; fi
    done
  fi
  [[ -z "$EVENT_KEY" ]] && return 0
  grep -o "\"$EVENT_KEY\"[[:space:]]*:[[:space:]]*\"[a-z_]*\"" "$f" \
    | sed -E 's/.*"([a-z_]+)"$/\1/'
}

ledger_of() { find "$1/.ctl" -name 'events.jsonl' 2>/dev/null | head -1; }

# Minimal dependency-free cargo crate under git. The "small deterministic
# fixture change" is a one-line bump in src/lib.rs (41 → 42). src/other.rs is a
# second tracked file OUTSIDE write_allow, used only by the deny probe.
setup_fixture() {
  git init -q
  git config user.email "harness@ctl.test"
  git config user.name "ctl-parity-harness"
  printf '[package]\nname = "fixture"\nversion = "0.0.0"\nedition = "2021"\n\n[lib]\npath = "src/lib.rs"\n' > Cargo.toml
  mkdir src
  printf 'pub fn answer() -> i32 { 41 }\n' > src/lib.rs
  printf 'pub fn unrelated() -> i32 { 0 }\n' > src/other.rs
  git add -A
  git commit -qm "fixture: baseline" >/dev/null
}

# ── HAPPY: identical full lifecycle through to archive ──
run_happy() {
  local A="$1" D="$OUT/$A/happy"
  rm -rf "$D"; mkdir -p "$D"; cd "$D"
  setup_fixture
  "$CTL" init >/dev/null 2>&1

  "$CTL" adapter doctor --json > "$D/doctor.before.json" 2>/dev/null || true

  "$CTL" task create --id happy \
    --objective "bump answer 41 -> 42 via $A" \
    --read-scope src --write-allow src/lib.rs --gates cargo_check > "$D/01_create.out" 2>&1
  "$CTL" task ready --id happy > "$D/02_ready.out" 2>&1
  "$CTL" task start --id happy > "$D/03_start.out" 2>&1
  "$CTL" run start --id happy --adapter "$A" > "$D/04_run_start.out" 2>&1; echo "exit=$?" >> "$D/04_run_start.out"

  # the deterministic fixture change + ACCEPTED in-scope evidence
  printf 'pub fn answer() -> i32 { 42 }\n' > src/lib.rs
  printf '{"source":"%s","touched_files":["src/lib.rs"]}' "$A" > "$D/accept_output.json"
  "$CTL" run ingest --id happy --adapter "$A" --result "$D/accept_output.json" > "$D/05_accept.out" 2>&1; echo "exit=$?" >> "$D/05_accept.out"

  "$CTL" task submit --id happy > "$D/06_submit.out" 2>&1
  git add -A; git commit -qm "fixture: bump answer 41 -> 42 ($A)" >/dev/null
  git rev-parse "HEAD^{tree}" > "$D/head_tree.txt" 2>/dev/null || true
  "$CTL" gate run --id happy --gate cargo_check > "$D/07_gate.out" 2>&1; echo "exit=$?" >> "$D/07_gate.out"
  CTL_ACTOR=ctl-review "$CTL" review accept --id happy \
    --note "parity e2e ($A): in-scope evidence source=$A; gate bound to committed tree" > "$D/08_audit.out" 2>&1; echo "exit=$?" >> "$D/08_audit.out"
  "$CTL" task finish --id happy > "$D/09_finish.out" 2>&1; echo "exit=$?" >> "$D/09_finish.out"
  "$CTL" task archive --id happy > "$D/10_archive.out" 2>&1; echo "exit=$?" >> "$D/10_archive.out"

  "$CTL" adapter doctor --json > "$D/doctor.after.json" 2>/dev/null || true

  local L; L="$(ledger_of "$D")"
  if [[ -n "$L" ]]; then cp "$L" "$D/events.jsonl"; event_kinds "$L" > "$D/lifecycle.txt"; fi
  grep -o '"source"[[:space:]]*:[[:space:]]*"[^"]*"' "$D/events.jsonl" 2>/dev/null | sort -u > "$D/evidence_sources.txt" || true
  grep -o '"tree_hash"[[:space:]]*:[[:space:]]*"[^"]*"' "$D/events.jsonl" 2>/dev/null | sort -u > "$D/tree_hashes.txt" || true
}

# ── DENY: isolated write-scope enforcement probe (deterministic) ──
run_deny() {
  local A="$1" D="$OUT/$A/deny"
  rm -rf "$D"; mkdir -p "$D"; cd "$D"
  setup_fixture
  "$CTL" init >/dev/null 2>&1

  "$CTL" task create --id deny \
    --objective "deny probe via $A" \
    --read-scope src --write-allow src/lib.rs --gates cargo_check > "$D/01_create.out" 2>&1
  "$CTL" task ready --id deny > "$D/02_ready.out" 2>&1
  "$CTL" task start --id deny > "$D/03_start.out" 2>&1
  "$CTL" run start --id deny --adapter "$A" > "$D/04_run_start.out" 2>&1; echo "exit=$?" >> "$D/04_run_start.out"

  # (a) ingest an in-tree but OUT-OF-write_allow file → SCOPE-001 evidence_rejected.
  printf '{"source":"%s","touched_files":["src/other.rs"]}' "$A" > "$D/deny_output.json"
  "$CTL" run ingest --id deny --adapter "$A" --result "$D/deny_output.json" > "$D/05_deny_ingest.out" 2>&1; echo "exit=$?" >> "$D/05_deny_ingest.out"
  # (b) live hook gate on the same out-of-scope path → allowed:false; control allows src/lib.rs.
  "$CTL" hook gate --tool write --path src/other.rs --task deny > "$D/06_hookgate_deny.json" 2>&1 || true
  "$CTL" hook gate --tool write --path src/lib.rs --task deny > "$D/07_hookgate_allow.json" 2>&1 || true
  # clean up the open run/worktree (non-fatal).
  "$CTL" run abort --id deny --reason "deny probe complete" > "$D/08_abort.out" 2>&1 || true

  local L; L="$(ledger_of "$D")"
  if [[ -n "$L" ]]; then cp "$L" "$D/events.jsonl"; event_kinds "$L" > "$D/lifecycle.txt"; fi
}

for A in "${ADAPTERS[@]}"; do
  echo ">>> $A: happy-path lifecycle"; run_happy "$A"
  echo ">>> $A: deny probe";          run_deny  "$A"
done

# ───────────────────────── PARITY COMPARISON ─────────────────────────
echo
echo "================= PARITY COMPARISON ================="
RC=0
A0="${ADAPTERS[0]}"; A1="${ADAPTERS[1]}"

pass() { echo "PARITY   $1"; }
fail() { echo "DIVERGE  $1"; RC=1; }

# 1. lifecycle event sequence (happy) identical across adapters.
if diff -q "$OUT/$A0/happy/lifecycle.txt" "$OUT/$A1/happy/lifecycle.txt" >/dev/null 2>&1; then
  pass "lifecycle (happy event sequence) identical"
else fail "lifecycle (happy event sequence) differs"; fi

# 2. adapter doctor: identical cross-adapter AND stable before==after.
diff -q "$OUT/$A0/happy/doctor.after.json" "$OUT/$A1/happy/doctor.after.json" >/dev/null 2>&1 \
  && pass "adapter doctor identical cross-adapter" || fail "adapter doctor differs cross-adapter"
for A in "${ADAPTERS[@]}"; do
  diff -q "$OUT/$A/happy/doctor.before.json" "$OUT/$A/happy/doctor.after.json" >/dev/null 2>&1 \
    && pass "adapter doctor before==after ($A)" || fail "adapter doctor before!=after ($A)"
done

# 3. evidence source — MUST differ (each adapter stamps its own tag).
echo "--- evidence source (intended divergence) ---"
s0=$(grep -c "\"$A0\"" "$OUT/$A0/happy/evidence_sources.txt" 2>/dev/null || echo 0)
s1=$(grep -c "\"$A1\"" "$OUT/$A1/happy/evidence_sources.txt" 2>/dev/null || echo 0)
echo "  $A0 evidence source tag present: $([[ "$s0" -ge 1 ]] && echo yes || echo NO)"
echo "  $A1 evidence source tag present: $([[ "$s1" -ge 1 ]] && echo yes || echo NO)"
if [[ "$s0" -ge 1 && "$s1" -ge 1 ]]; then pass "each adapter stamps its own source tag"; else fail "source tag missing"; fi

# 4. gate binding — gate/audit tree_hash equals that run's HEAD^{tree}.
echo "--- gate binding (tree_hash == HEAD^{tree}) ---"
for A in "${ADAPTERS[@]}"; do
  ht="$(cat "$OUT/$A/happy/head_tree.txt" 2>/dev/null)"
  if [[ -n "$ht" ]] && grep -q "$ht" "$OUT/$A/happy/tree_hashes.txt" 2>/dev/null; then
    echo "  $A bound: HEAD^{tree}=${ht:0:12}"
  else echo "  $A UNBOUND: HEAD^{tree}=${ht:0:12}"; RC=1; fi
done

# 5. finish/archive — both exit=0.
echo "--- finish/archive (must be exit=0) ---"
for A in "${ADAPTERS[@]}"; do
  fin="$(grep -o 'exit=[0-9]*' "$OUT/$A/happy/09_finish.out" | tail -1)"
  arc="$(grep -o 'exit=[0-9]*' "$OUT/$A/happy/10_archive.out" | tail -1)"
  echo "  $A finish=$fin archive=$arc"
  [[ "$fin" == "exit=0" && "$arc" == "exit=0" ]] || RC=1
done

# 6. write-scope deny — out-of-scope ingest non-zero + evidence_rejected on ledger
#    + hook gate allowed:false, identically for both adapters.
echo "--- write-scope deny (deterministic) ---"
for A in "${ADAPTERS[@]}"; do
  dny="$(grep -o 'exit=[0-9]*' "$OUT/$A/deny/05_deny_ingest.out" | tail -1)"
  rej="$(grep -c 'evidence_rejected' "$OUT/$A/deny/lifecycle.txt" 2>/dev/null || echo 0)"
  hg="$(grep -o '"allowed"[^,]*' "$OUT/$A/deny/06_hookgate_deny.json" 2>/dev/null | head -1)"
  ha="$(grep -o '"allowed"[^,]*' "$OUT/$A/deny/07_hookgate_allow.json" 2>/dev/null | head -1)"
  echo "  $A ingest=$dny evidence_rejected=$rej hookgate(out)=$hg hookgate(in)=$ha"
  [[ "$dny" != "exit=0" && "$rej" -ge 1 ]] || RC=1
  echo "$hg" | grep -q false || RC=1
  echo "$ha" | grep -q true  || RC=1
done

echo
echo "OUT_DIR=$OUT"
if [[ "$RC" == "0" ]]; then
  echo "RESULT: PARITY — governance identical across adapters; only the evidence source tag differs, by design."
else
  echo "RESULT: DIVERGENCE DETECTED — inspect OUT_DIR."
fi
exit "$RC"
