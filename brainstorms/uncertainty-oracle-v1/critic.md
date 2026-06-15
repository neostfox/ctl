# Critic — uncertainty-oracle-v1 (BS-UO1)

> L0 content. Independent challenge pass over `divergence.md`, produced by a separate
> read-only review agent that was instructed to refute, then triaged by the originator.
> **Independence is `unattested`**: same model family, no trusted orchestrator, no
> authenticated principal (EPISTEMIC_CONTROL §2). The canonical event records only that
> a critic artifact exists — never that the challenge was truly independent or correct.

## Method

A read-only critic agent read `divergence.md`, `EPISTEMIC_CONTROL.md`, the existing
uncertainty ledger (`src/domain/task.rs` 217–490, 1452–1558), `src/application/mod.rs`
895–1085, and the protected schema (1309–1446), and was told to find flaws and cite
`file:line`. Below: each objection, then the originator's disposition. The *delta*
(what the critic changed) is the real evidence here, not the agreement.

## Objections raised → dispositions

**C1. Dual-path resolve can smuggle BOTH `evidence_ref` and inline `evidence_path`.**
The reducer decodes inline evidence unconditionally; nothing rejects an event carrying
both, so a crafted event could bind a second evidence under the canonical wrapper.
→ **ACCEPTED.** New invariant: a `resolved` disposition may carry *either* `evidence_ref`
(new) *or* inline `evidence_path`+`evidence_hash` (legacy), **never both** — reducer
rejects "both". Reducer is the authority; the schema also encodes the mutual exclusion
where cheap. This is a genuine design change from the divergence (which left it open).

**C2. Schema rollback / `additionalProperties:false` breaks old validators; bump to v2
or a new event type.**
→ **PARTIALLY REJECTED, with the valid kernel kept.** We do not maintain forward-compat
with *pre-feature* validators (every prior feature — bs-prov, uncertainty-ledger,
research-spike — added optional fields + enum members to this same schema in place; no
v2). The committed disposition events carry no `evidence_ref`, so they still validate
against the extended schema because the new fields are *optional*. Adding the
`evidence_recorded` enum member + its allOf block cannot invalidate any existing event
(none use it). The kernel I keep: the new fields MUST be declared optional in
`properties` (never required), and I will add a replay test over the real legacy stream.
A v2 envelope would contradict EPISTEMIC_CONTROL §8's "只引入三种事件" minimalism.

**C3. A separate `recorded_by` payload field is a forgeable second principal that can
contradict the envelope actor (EPISTEMIC_CONTROL §2).**
→ **ACCEPTED — this resolves divergence open-question 5.** `recorded_by` is NOT a
caller-supplied payload field. It is captured from the envelope `actor` at record time
and stored on the Evidence for per-item disclosure, shown as
`recorded_by: <actor> (unattested principal)`. One principal, not two. The task-spec
field `recorded_by` is satisfied without inventing a contradictory identity.

**C4. `model`-oracle evidence leaks into the aggregate "resolved with evidence: N" line
(src/cli/mod.rs:2034), which acoustically implies external proof.**
→ **ACCEPTED as a disclosure requirement, not a new hard rule.** V1 still allows any
`oracle_kind` to back a `resolved` (the spec lists `model` as a valid kind and only
requires resolved⇒evidence_ref; coupling them further would exceed scope). Honesty is
carried by: (a) an `ORACLE SOURCES` breakdown with `model advisory: N` as its own line;
(b) every resolved item rendering its `oracle_kind`; (c) `model` items carrying an
explicit `ADVISORY — not external proof` marker; (d) no score/ratio/verdict. The
existing aggregate line stays (the task spec §五 shows it verbatim) but can no longer
stand alone.

**C5. Adding UNCERTAINTIES to `ctl task status` risks a green-check by adjacency to gate
PASS lines.**
→ **ACCEPTED (partial).** The task-status block carries the same
`(content: unverified; evidence: unattested)` banner as `ctl uncertainty status`, plus
the ORACLE SOURCES breakdown, all fact-only. I will NOT add an editorial preamble
sentence (editorializing is itself a soft verdict); the banner + per-kind textures + the
absence of any total/score are the mitigation. The block is purely additive and renders
for all task kinds when uncertainties exist.

**C6. `tree_hash`-only evidence invites a content-free "runtime"/"deterministic" oracle
that is pure assertion.**
→ **ACCEPTED — DROP `tree_hash` from V1.** Every evidence requires a file-backed
artifact (`artifact_path` + ctl-computed hash, reusing `hash_evidence`). If runtime or
deterministic evidence exists, the producer saves its output to a file and records that;
if no artifact exists, the correct disposition is `accepted_as_assumption`, not
`resolved`. `oracle_kind` labels *what kind* of oracle produced the artifact;
`source_ref` is a free-text, unattested locator (command, test name, URL). This tightens
the divergence's D6 (which had offered artifact OR tree_hash).

**C7/C8. `evidence_ref → recorded evidence` is a reducer invariant, not a schema one;
forward references / cryptic late errors.**
→ **ACCEPTED as documented invariant.** The per-task ledger is a single-writer, seq-
ordered stream (hardened in commit 2b3a7a5). Events apply in seq order, so an
`evidence_recorded` referenced by a later `uncertainty_disposition_recorded` is already
in state; a disposition naming an unknown `evidence_id` is rejected with a clear message
(mirroring the existing "unknown uncertainty" rejection). Schema cannot express a cross-
event foreign key; the reducer is the authority. CLI always records evidence first.

## What the critic could NOT refute (kept from divergence)

- The motivation (EPISTEMIC_CONTROL §5.1 + §8 demand oracle-typing of evidence).
- Record-and-disclose boundary: no scoring, no verdict, no gating on oracle kind.
- The fixed `oracle_kind` enum (no free string, no `other`).
- The `model = advisory` pin.
- The fact-only uncertainty-ledger rendering as the template to extend.

## Net design changes the critic forced

1. Reject "both evidence_ref and inline evidence" on a resolve (C1).
2. `recorded_by` = envelope actor, never a separate forgeable field (C3).
3. Split model out in disclosure; per-item oracle_kind + advisory marker (C4).
4. Drop `tree_hash`; require a file artifact for every evidence (C6).
5. Add an explicit legacy-stream replay test (C2 kernel).
