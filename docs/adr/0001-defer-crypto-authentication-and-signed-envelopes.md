# ADR 0001 — Defer cryptographic authentication & signed orchestration envelopes

- **Status:** Accepted (Deferred)
- **Priority:** **LOWEST** — conditional / icebox. Do NOT pick up without a
  triggering context change (see *Revisit triggers*).
- **Date:** 2026-06-20
- **Affected roadmap items:** `authenticated-principal-v1`,
  `trusted-orchestrator-envelope-v1`
- **Source:** the orchestration-trust audit (`brainstorms/orchestration-trust-audit-v1.md`);
  follows the honesty remediation shipped in ctl 0.0.5.

## Context

The orchestration-trust audit found a gap between ctl's strong vocabulary
("trusted orchestrator", "authenticated principal", "attestation") and its actual
mechanism (advisory PreToolUse hooks + `CTL_ACTOR` string labels +
record-and-disclose evidence). Two remedies exist for such a gap:

1. **Soften the words** to match the mechanism, or
2. **Build the strong mechanism** (cryptographic identity + signing) to match the
   words.

Remedy (1) is **already done** in 0.0.5 — the docs now state "actor-label
separation, NOT an authenticated principal" and "record-and-disclose, NOT proof".
So the *dishonesty* the audit flagged is resolved. The two remaining items are
remedy (2):

- **`authenticated-principal-v1`** — replace the actor-label reviewer≠implementer
  interlock (a forgeable `CTL_ACTOR` string) with a verified principal identity:
  local keystore / signature / OIDC subject, a principal registry, role policy,
  and signed audit events.
- **`trusted-orchestrator-envelope-v1`** — have ctl issue signed run/subagent
  envelopes carrying scope, lease, role, budget, and hashes.

## Decision

**Do NOT build either item now. Keep both as disclosed, condition-gated roadmap at
the lowest priority.** Honest disclosure is the correct and sufficient response to
the audit for ctl's current product: a local-first, single-user, trusted-operator
dev tool.

## Rationale (threat model first)

Cryptographic identity and signed envelopes only buy something against an
**untrusted party that can run processes / set env vars / write files** — a
malicious or compromised agent forging approvals, an insider tampering with the
ledger, or multiple mutually-distrusting parties on a shared/remote control plane.

ctl is none of those today: it is **local, single-user, and trusts the operator**
(README/DESIGN: "NOT a daemon / web service / remote orchestration"). The
governance exists to catch *mistakes* (scope drift, hallucinated specs, unreviewed
changes), not to defend against a cryptographic adversary.

Two decisive points:

- **On a local single-user machine, crypto likely does NOT deliver the property it
  promises.** The agent runs with the operator's filesystem permissions, so it can
  read any local signing key and sign as "the reviewer" — exactly the forgery the
  actor label already permits, now with more machinery. Complexity bought, no real
  adversary-resistance gained.
- **It would re-introduce overclaiming.** Shipping "now cryptographically
  authenticated" on a substrate where the key is locally readable is the same
  word-vs-mechanism dishonesty the audit set out to remove.

## Consequences

**Of deferring (chosen):**

- Low risk now — the honesty work already removed the misleading claims.
- The reviewer≠implementer "hard gate" remains *label* strength: a solo agent can
  play both roles (recording audits under `CTL_ACTOR=ctl-review` while also
  implementing). Acceptable for a solo, trusted-operator workflow, and honestly
  disclosed.

**Of building (rejected for now):**

- Breaks ctl's architectural identity: signing needs a crypto crate and OIDC needs
  an HTTP client — both forbidden by `AGENTS.md` (deps capped at
  `clap`/`serde`/`anyhow`/`sha2`/`libc`; no HTTP client; no async/daemon).
- Breaks determinism / replayability: signatures + timestamps + nonces make events
  non-deterministic, undermining the pure-reducer, replayable ledger.
- High complexity (key management, rotation) for a property not actually delivered
  locally (see Rationale). OIDC in particular has no coherent meaning for a local
  CLI.

## Revisit triggers (only then is this worth re-opening)

Re-open ONLY if ctl's context changes such that the trusted-operator assumption no
longer holds:

1. ctl becomes a **shared / remote / multi-party** control plane (a server,
   multiple untrusted clients).
2. You decide to **treat the agent as untrusted** (adversarial-agent threat model),
   with key material held outside the agent's reach (HSM / separate trust domain).
3. An **external compliance / audit** requirement demands tamper-evident,
   identity-bound approvals that an outside party (who does not trust the operator)
   can verify.

If/when triggered, run a first-principles alignment pass first: which exactly-one
crate, whether the no-HTTP / no-async / determinism hard-stops are amended
deliberately (this ADR then superseded), and where keys live so the property is
real.

## Note — if you only want tamper-EVIDENCE, this is the wrong tool

"Detect that the ledger was edited after the fact" does **not** require
authenticated principals or OIDC. It needs an **L3 hash-chain** (sha2-based —
already an allowed dep), a much lighter, identity-free path. Identity (*who*) and
authorization (*signed envelope*) only matter under the untrusted-party threat
model above; integrity (*not modified*) has a cheaper implementation. Keep these
concerns separate when prioritizing.
