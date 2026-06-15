# Critic — Research/Spike V1

> Independent critic artifact. L0 content. Task: `research-spike-v1`.
> Adversarial pass over `brainstorms/research-spike-v1/divergence.md`, grounded in the shipped
> reducer (`src/domain/task.rs`) and application layer (`src/application/mod.rs`). This is a
> challenge, not a sign-off. Where I agree with the originator I say so explicitly so convergence
> can tell agreement from un-reviewed acceptance. Produced in a separate context; independence is
> **unattested** (no trusted orchestrator — §2/§8).

## The single biggest weakness

The completion bar does not enforce anything the §3 product principle actually cares about, and the divergence artifact knows it but waves it through under "quality is unjudged anyway." Those are two different claims. §6/§9 forbid judging *quality*; they do not forbid the gate being *honest about what it observed*. The proposed bar — `≥1 research_artifact_recorded AND ≥1 uncertainty outcome` — binds neither the artifact to the work nor the uncertainty to the artifact. A research task can complete by recording one zero-byte `findings.md` (hash bound, content arbitrary) and one open uncertainty `U-1: "unclear"`. That passes every check. The control plane then truthfully discloses "1 artifact, 1 open uncertainty" — and the disclosure is *correct*. The weakness is not that the bar is gameable; it is that **the bar is not measuring the thing the feature exists to make observable**: that a spike produced evidence and touched its uncertainty map. It measures that two events of the right *type* exist. That is a type-presence check wearing a completion-semantics costume. See my position on the bar below — I do not think the fix is a higher bar; it is dropping the pretense that this is a "bar" at all.

## A factual error that breaks Axis C and Q3

The divergence artifact's Axis C and open-question 3 are built on a false premise about the shipped reducer. They assume "multiple reopens produce multiple `task_started` events." **They do not.** In `src/domain/task.rs:624`, `task_started` is guarded `Can only start from Ready`; it fires exactly once, on the Ready→InProgress edge. Reopen (`src/application/mod.rs:307`, reducer at `task.rs:642`) emits `task_reopened` on the Review→InProgress edge — a *different* event type. `task_started` therefore never repeats; "first" vs "latest" `task_started` is a distinction without a difference because there is only ever one.

This is not a nitpick. It means:
- Q3 as posed ("which `task_started` boundary, can a reopen reset the count?") is answering a question the system cannot ask. The real boundary question is whether "during task" should advance to the latest `task_reopened` — a question the artifact never poses.
- The "first `task_started`" rule in Axis C is accidentally correct *for the opened-vs-discovered split*, but for the wrong reason, and it leaves the reopen semantics completely unspecified.

Convergence must rewrite Axis C against the actual event vocabulary before deciding anything.

## The five open questions — concrete positions

**Q1 — Is `≥1 artifact + ≥1 uncertainty outcome` meaningful, or trivially gamed; acceptable for V1?**
Trivially gamed, and "trivially gamed but honest" is acceptable for V1 *only if you stop calling it a completion bar and call it a completion shape*. My position: keep the two structural requirements (they prevent the degenerate "research task that completed with literally no epistemic or evidentiary footprint," which is the one case worth blocking), but the artifact must drop all language implying the bar screens for spike quality or effort. The bar's sole defensible job is: a research task may not complete looking *identical* to an implementation task that produced nothing. It does that. Anything more is a quality judgment §9 forbids. Accept it — with the framing corrected and with change C2 below.

**Q2 — Is `task_kind` immutability right?**
Right. Immutable. Defended in detail in the dedicated section below. There is no legitimate reclassification case that a *new task* doesn't serve better, and every mutable variant opens a concrete integrity hole.

**Q3 — Under multiple reopens, which seq boundary?**
The question is malformed (see factual error above): there are no multiple `task_started` events. Corrected position: derive "discovered during task" as `uncertainty_recorded` events with `seq >` the single `task_started` seq. Do **not** advance the boundary on `task_reopened`. Rationale: advancing it would let a reopen retroactively reclassify previously-discovered uncertainties as pre-existing, shrinking the discovered count — the exact gaming the artifact feared, just via the wrong event. Anchoring to the one immutable `task_started` seq is monotonic and ungameable. But see the Planning edge case below — the boundary is necessary but not sufficient.

**Q4 — Does "new uncertainties discovered: N" become a covert verdict?**
Yes, structurally, and neutral wording is **not** enough. Detailed below in the dedicated Q4 section. Something structural must change.

**Q5 — `artifact_kind`: fixed enum vs free string?**
Fixed enum, but the proposed four values are wrong as a closed set. `findings | experiment | recommendation | design_draft` conflates two axes: *what the artifact is* (a document) vs *what epistemic role it plays*. `experiment` is not a peer of `findings` — an experiment produces findings. My position: keep a fixed enum (free strings invite the very taxonomy-that-pretends-to-be-meaningful §6 warns against, and the disposition enum precedent at `task.rs:241` is the right discipline), but ship a smaller, orthogonal set — `findings | recommendation | design_draft` — plus **no `other`**. Drop `experiment`: there is no experiment runner in scope (§9), so an `experiment` artifact_kind is an aspirational label with no machinery behind it — precisely the "lab coat on a green check" §3 warns against. If a spike ran an experiment, its output *is* `findings`. Adding `other` reintroduces the free string through the back door; refuse it for V1 and let the absence be a pressure signal for V2.

## Q4 in depth — the covert-metric problem

Disclosing `new uncertainties discovered: N` as a raw count does not escape §6 merely by refusing to label it. §6's actual claim is subtler than "don't print a verdict": it says any epistemic dimension, *once given a number a reader can rank on*, gets read as a green/yellow/red light and then gets reverse-gamed. A monotonic integer the reader can compare across spikes (`spike A: 0 discovered, spike B: 7 discovered`) is rankable. Two failure directions, both real:
- **Read as "good spike" (high N = thorough).** Incentivizes splitting one unknown into seven `uncertainty_recorded` events to inflate the count. The reducer happily accepts seven distinct ids (`task.rs:1312` only dedups by exact id).
- **Read as "bad spike" (high N = the work made things worse / opened a mess).** Incentivizes *not recording* discovered unknowns, which is the precise harm the whole Uncertainty Ledger exists to prevent. This is the dangerous one: a covert "fewer is better" reading silently recreates the fewer-unknowns success metric §9 explicitly forbids and the originator's own Problem statement calls out ("a spike that opens four hidden risks is a success").

Neutral wording ("disclosed as a neutral fact, never colored") is a UI promise, not a structural property. The number is still rankable the moment it's an integer. **Structural change required (see C1):** do not surface "new discovered" as a standalone scalar at all. Surface the discovered uncertainties only as *items* interleaved in the same fact-only list as all other uncertainties, each tagged with its disposition, with no separate count line and no separate "discovered" subtotal. A reader who wants to know "what did this spike surface" reads the items; there is no scalar to rank. If convergence insists on machine-readability, expose it only in the structured view as a derived field that is *never* rendered as a headline number in the human `RESEARCH OUTPUT` block. The artifact currently proposes the opposite (G1 lists "new discovered during task" as a first-class count). That must change.

## The completion bar — clear position

Meaningful: no. Trivially gamed: yes — one empty file + one open unknown clears it, and there is no honest way to make it un-gameable without judging quality, which is forbidden. "Trivially gamed but honest" is **acceptable for V1**, conditionally. The condition is that the artifact stops overselling it. Right now Axis B reads as if the two checks are a *substantive* completion criterion ("completion is defined by evidence + epistemic outcomes"). They are not; they are a *minimum non-degeneracy shape*. Reframe accordingly. The one thing I will defend the bar against is *removing* it: without `≥1 artifact`, a research task degenerates into "implementation task that happens to skip code review," and the §9 line about not exempting spikes from execution integrity gets quietly eroded. Keep the two checks; demote the rhetoric.

One concrete gap the bar leaves open that the artifact does not mention: it requires `≥1 uncertainty outcome` defined as "a recorded uncertainty OR a disposition," but a disposition with no prior `uncertainty_recorded` is impossible (the reducer at `task.rs:1333` rejects dispositions for unknown ids). So "≥1 outcome" collapses to "≥1 `uncertainty_recorded` event ever." State that plainly — the disjunction in the directive is misleading.

## `task_kind` immutability — defend, with the concrete failure each alternative invites

Defend immutability. Each mutable choice invites a *named* integrity failure:
- **`research → implementation` at finish time** dodges the artifact requirement: an agent does spike-shaped work, produces no code, and flips kind to complete via the implementation path with no `research_artifact_recorded`. The feature's only enforcement evaporates at the last gate.
- **`implementation → research` at finish time** dodges code review and gate binding *as a category*: real code ships, then the task is reclassified so the disclosure block reads as a spike, laundering "I never passed the gates" into "I'm a research task, gates apply but the framing is epistemic." Even though §B says gates still apply, the *framing* and the reviewer's expectations shift, and the actor can satisfy the research bar (one findings.md describing the code) instead of producing passing gates if any gate was previously waived.
- **Mutable-but-only-in-Planning** is the seductive middle and still fails: it interacts with the Planning-phase uncertainty edge case below, and it means `task_kind` is not part of task identity at creation, which breaks the clean replay-default story (`#[serde(default)] → Implementation`) the originator correctly relies on.

The legitimate reclassification need ("I thought this was implementation, it's actually a spike") is real but is correctly served by **cancel + create new task**, which preserves an honest audit trail (one cancelled implementation task, one new research task) instead of mutating identity. Immutable. The originator got this right.

## The seq-boundary + Planning edge case — this breaks the opened-vs-discovered distinction

The artifact's own Axis C notes "a task may record uncertainties while still in Planning, before start" and treats it as handled by the `seq > task_started` rule. It is handled *for the split* but it exposes a deeper problem the artifact misses: **the reducer places no phase guard on `uncertainty_recorded`** (`task.rs:1307` — no phase check at all, unlike `task_started` at :625 or `task_submitted_for_review` at :633). Consequences:
- A research task can record all its uncertainties in **Planning**, then go Ready, start, submit — and they all count as "opened, not discovered."
- An agent that wants a clean "0 discovered" disclosure simply front-loads every `uncertainty_recorded` before calling `start`. The split is fully gameable by *reordering*, with zero dishonesty detectable at the event level. This is a second, independent reason (beyond Q4) to **not surface a discovered count**: the count is not just rankable, it is *manufacturable by sequencing alone*. The opened-vs-discovered distinction is epistemically meaningless under a reducer that lets you record uncertainties in any phase. Either drop the distinction from the disclosure entirely (my preference, folds into C1), or add a phase guard — and a phase guard is a behavior change to shipped Uncertainty Ledger V1, which is out of scope and would be the wrong fix anyway.

## At least two concrete changes before convergence

**C1 — Remove "new uncertainties discovered: N" as a scalar from `RESEARCH OUTPUT`.** Surface discovered uncertainties only as items in the existing fact-only uncertainty list (reuse `UncertaintyItemView`, `task.rs:311`). No subtotal, no count line, no derived "discovered" headline. Optionally retain a machine-only derived field in the structured view, never human-rendered. This neutralizes both the covert-metric reading (Q4) and the sequence-manufacturability (Planning edge case) in one move, and it shrinks scope.

**C2 — Bind the artifact freshness in the disclosure, exactly as the Uncertainty Ledger already binds evidence freshness.** `research_artifact_recorded` reuses `ArtifactRef`; the disclosure must run the same `Current/Stale/Absent` freshness derivation the ledger uses (`uncertainty_ledger_view`, `mod.rs:962-972`) so a recorded artifact whose file was deleted or mutated after recording shows `STALE/ABSENT`. Without this, the artifact hash is recorded at completion and never re-checked — a research task can complete pointing at a `findings.md` that no longer exists or has been rewritten, and the disclosure would still read "1 artifact." Reusing the shipped freshness primitive is in-scope, cheap, and closes the one genuinely misleading state.

**C3 (additional) — Rewrite Axis C against real event vocabulary.** Anchor "during task" to the single `task_started` seq; explicitly state reopen (`task_reopened`) does NOT advance the boundary and does NOT re-emit `task_started`. Remove all "first vs latest `task_started`" language. If C1 lands, this collapses to a one-line internal note.

## One thing the originator got right (defend against scope creep)

**D1 — one event, no `research_completed`.** This is correct and must be held against the inevitable convergence pressure to add a summary event "for convenience." A `research_completed` event would (a) duplicate the derivable artifact+uncertainty state, (b) read as "research was sufficiently done" — the exact `brainstorm_completed` compression §7 explicitly rejects, and (c) become a snapshot that drifts from the lazily-derived truth, violating the "派生量不持久化" discipline (§5.2). Deriving the `RESEARCH OUTPUT` purely at read time from canonical artifact + uncertainty events is the right call and is consistent with how the shipped ledger already works. Defend this hard. Likewise the originator is right to reuse `ArtifactRef` and pin `content_l0` / `attestation: unavailable` (F1) — no new trust model, §9-clean.

## Revised uncertainty set

Open unknowns I believe remain after this critique (for convergence to dispose, not for V1 to necessarily solve):

- **U-R1 (blocking, should resolve before convergence):** Should the "discovered during task" concept survive at all in V1 disclosure, given it is both covert-metric-prone (Q4) and manufacturable by sequencing (Planning edge case)? My lean: drop it via C1. *Open.*
- **U-R2:** Does removing the scalar (C1) leave any legitimate consumer un-served (e.g., a future risk-driven externalize-before-implement workflow, §4 roadmap item) that genuinely needs a discovered count? If so, where does that count live without being rankable? *Open.*
- **U-R3:** Is the reducer's lack of a phase guard on `uncertainty_recorded` a latent defect in shipped Uncertainty Ledger V1 (independent of this feature), or intended? Out of scope to fix here, but should be recorded as a known uncertainty against the ledger, not silently relied upon.
- **U-R4:** What is the exact `artifact_kind` set after dropping `experiment` and refusing `other`? Confirm `findings | recommendation | design_draft` covers V1's real cases without an escape hatch, and document that the absence of `experiment`/`other` is a deliberate V2 pressure signal.
- **U-R5:** Under reopen, an agent can record a *new* artifact and a *new* uncertainty post-reopen and re-finish. Confirm the two structural checks are over all events for the task (cumulative), not "since last reopen," and that this matches the shipped freshness/gate-staleness behavior on reopen. *Open.*
- **U-R6:** `source_run_id?` on `research_artifact_recorded` is optional and unattested (same status as `critic_run_id` per §2/§8). Confirm the disclosure renders it as an unattested claim, never as provenance — otherwise it becomes the "structurally-better self-report" §2 warns against.
