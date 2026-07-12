---
name: ctl-decision-map
description: "Maintain a project-level decision map for an effort too big to plan upfront — one that grill found has fog (decisions blocked on frontier work advancing). The map is an index, not a store: it tracks Destination, Frontier (ctl task links), Fog (unresolved decisions), and Out of scope; fog graduates into tasks incrementally via ctl-to-tasks rather than one upfront slice pass. Lives at .ctl/spec/maps/<slug>.md (spec tier, human-writable). Triggers when: grill surfaces fog during alignment on a large effort, or a multi-session effort re-opens and needs re-orientation. Do NOT trigger for: a no-fog effort (go straight to ctl-to-tasks), a single well-scoped task, or anything that fits one session."
---

# ctl-decision-map (Claude Code)

The **managed core** below is the platform-neutral ctl workflow protocol, byte-checked by CI against `.agent/protocols/workflow-skills.md` across platforms. Do not edit it here — it is generated from `.agent/skills/ctl-decision-map/source.md` by `ctl skills sync`. Claude Code-specific mechanics live after the core.

<!-- ctl:workflow-core:start version=1 -->
# ctl Workflow Skills — Core Protocol

WORKFLOW_PROTOCOL_VERSION = 1

This is the platform-neutral workflow-skills core. It is split into an
**embedded** part (division of labor + invariants), carried verbatim inside
every workflow skill's managed-core block, and a **reference** part (phase map +
frameworks + provenance) that lives only in this file — the auto-loaded
control-guard carries the pipeline routing, and each skill's body covers its own
phase. The canonical copy lives at `.agent/protocols/workflow-skills.md`; a CI
drift check fails if any embedded copy diverges. Edit this file and re-sync
every workflow skill together — never one in isolation. Nothing platform-specific
(tool names, hook mechanics, plugin paths) and nothing phase-specific belongs in
the embedded part; that lives in each skill outside the managed core.

## Division of labor (non-negotiable)

Skills and agents manage **semantic workflow** — what to think about, in what
order, and which artifact each phase produces. ctl manages **facts, scope,
evidence, gates, ledgers, and honest disclosure**. A workflow skill never relaxes
a boundary, never declares a task complete, and never substitutes its own
judgement for ctl evidence. Workflow discipline is not proof: it does not replace
gates, audits, reviewer independence, or tamper evidence, and it never creates a
verdict.

## Invariants every phase honors

- Produce **artifacts, not claims**. "Done" is an evidence artifact ctl can see,
  never an assertion — "where is the evidence?"
- Keep **draft separate from confirmed basis**; disclose open uncertainty rather
  than hiding it.
- **Red before green**: no green claim without prior red evidence for the same
  behavior.
- **No fix before a reproduction loop.**
- **Architecture review is read-only**; a refactor needs a fresh governed task.
- External workflow inspiration is **L0 reference material** (see Provenance) —
  never an authority, never vendored as an active control.
<!-- ctl:workflow-core:end -->

*The phase map, frameworks, and provenance are reference material in `.agent/protocols/workflow-skills.md` — not embedded here. The auto-loaded control-guard carries the pipeline routing; this skill's body covers its own phase.*

## Station contract

- **Upstream**: `ctl-grill-with-spec` — when alignment finds an effort has fog
  (decisions that can't resolve until the frontier advances), it seeds a map here
  instead of converging to a single upfront `ctl-to-tasks` pass.
- **Produces**: a decision map at `.ctl/spec/maps/<slug>.md` (status: `draft` →
  `living` while fog remains → `closed` when Destination is reached and no fog
  remains).
- **Downstream**: graduation — each resolved fog item hands off to `ctl-to-tasks`
  (or `ctl task create` for a trivial slice); the new task ID re-enters Frontier.

## When to build a map — and when not to

Read [`decision-map.md`](../../spec/guides/decision-map.md) first. It defines the
format, sections, and graduation rule.

Build a map **only** when grill surfaces fog. The test: are there decisions that
cannot be made until work on the frontier advances? If yes, the effort is too big
to plan upfront — build a map. If every decision resolves in the alignment
interview, the effort fits one session: **go straight to `ctl-to-tasks`** and skip
the map. A no-fog effort gets no map.

## The map discipline

**The map is an index, not a store.** A decision lives in exactly one place — its
ctl task once it graduates. The map gists and links; it never restates a task's
objective or scope (that is what `ctl board` and the task itself are for).

Four sections (see the guide for the full schema):

- **Destination** — the outcome that must be true when done. One sentence.
- **Frontier** — ctl task IDs ready/active; links only, no restatement.
- **Fog** — decisions blocked on frontier work. Each names: the decision · its
  blocker · graduating `kind` (implementation/research) · AFK/HITL.
- **Out of scope** — ruled beyond the destination. Closed, never graduates.

### Graduation (the core mechanic)

```
fog item  --blocker completes, decision resolves (HITL via grill if needed)-->
          ctl-to-tasks slice (or ctl task create)
          --> new task ID enters Frontier
          --> fog item struck from the map
```

Nothing lingers in two places. Graduating fog clears the patch.

### The per-session loop

1. Open the map; re-read **Destination**.
2. `ctl board` / `ctl next-task` confirm the takeable Frontier; claim or resume one.
3. Work it under normal governance (scope, gates, evidence).
4. New decisions found while working → add to **Fog** with their blocker.
   Ruled-out work → **Out of scope**.
5. A completed frontier task may unblock fog → resolve and graduate.
6. **Close** when Destination is reached and no fog remains.

## Anti-patterns

- ❌ Restating a task's objective/scope in the map instead of linking its ID
  (the map is an index, not a store).
- ❌ Building a map for a no-fog effort that fits one session.
- ❌ Letting a decision live in both the map's Fog *and* a graduated task —
  graduate fully: strike the fog.
- ❌ Treating Out-of-scope as deferred Fog — out-of-scope is ruled out, never
  graduates.
- ❌ Letting the map drift from `ctl board` — the Frontier mirrors the machine
  view; if they disagree, the task ledger is truth, the map follows.
- ❌ Using the map as a substitute for governance — it orients a human; it does
  not replace gates, scope, evidence, or `ctl task` lifecycle.

## Provenance

Inspired by Matt Pocock's `wayfinder` skill (v1.1) — the fog-of-war / frontier /
"map is an index, not a store" framing — adapted to ctl's governed-task model:
the map links into ctl tasks and graduates fog via `ctl-to-tasks`, rather than
managing its own ticket tracker. External skill text is L0 reference material;
this is a ctl-native rewrite, not a vendored control.

## Claude Code Integration (platform-specific)

The map lives at `.ctl/spec/maps/<slug>.md` (spec tier — writable under the
gate). Seed from the confirmed alignment note. Run the per-session loop inline so
writes carry the active task's `CTL_TASK_ID` binding when one exists; read-only
Frontier checks (`ctl board`, `ctl next-task`) can be dispatched to a subagent
(built-in `Explore`, `claude-code-guide`). Graduation (`ctl-to-tasks`,
`ctl task create`) goes through normal governance.
