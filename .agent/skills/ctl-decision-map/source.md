---
name: ctl-decision-map
description: "Maintain a project-level decision map (Destination / Frontier / Fog / Out-of-scope) for an effort too big to plan upfront — one grill found has fog. Fog graduates into tasks incrementally as the frontier advances. Triggers when: grill surfaces fog during alignment on a large effort, or a multi-session effort re-opens needing re-orientation. Do NOT trigger for: a no-fog effort (go straight to ctl-to-tasks), a single well-scoped task, or anything that fits one session."
---


## Station contract

- **Upstream**: `ctl-grill-with-spec` seeds a map when alignment finds fog.
- **Produces**: a decision map at `.ctl/spec/maps/<slug>.md` (`draft` → `living` → `closed`).
- **Downstream**: each resolved fog item graduates to `ctl-to-tasks` (or `ctl task create`); the new task ID re-enters Frontier.

## When to build a map

Read [`decision-map.md`](../../spec/guides/decision-map.md) for the full schema. Build a map **only** when grill surfaces fog — decisions that cannot be made until the frontier advances. If every decision resolves in the alignment interview, the effort fits one session: **go straight to `ctl-to-tasks`**. A no-fog effort gets no map.

## The map discipline

**The map is an index, not a store.** A decision lives in exactly one place — its ctl task once it graduates. The map gists and links; it never restates a task's objective or scope.

Four sections (see the guide for the full schema):

- **Destination** — the outcome that must be true when done. One sentence.
- **Frontier** — ctl task IDs ready/active; links only, no restatement.
- **Fog** — decisions blocked on frontier work. Each names: the decision · its blocker · graduating `kind` (implementation/research) · AFK/HITL.
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
4. New decisions found while working → add to **Fog** with their blocker. Ruled-out work → **Out of scope**.
5. A completed frontier task may unblock fog → resolve and graduate.
6. **Close** when Destination is reached and no fog remains.

## Anti-patterns

- ❌ Letting the map drift from `ctl board` — the task ledger is truth; the map follows.
- ❌ Using the map as a substitute for governance — it orients a human; it does not replace gates, scope, evidence, or `ctl task` lifecycle.

## Provenance

Inspired by Matt Pocock's `wayfinder` skill (v1.1) — the fog-of-war / frontier / "map is an index, not a store" framing — adapted to ctl's governed-task model: the map links into ctl tasks and graduates fog via `ctl-to-tasks`. External skill text is L0 reference material; this is a ctl-native rewrite, not a vendored control.

<!-- integration:omp -->

The map lives at `.ctl/spec/maps/<slug>.md` (spec tier — writable under the OMP
PreToolUse ctl gate; protected paths remain hard-denied). Seed it from the
confirmed alignment note. Graduation writes (`ctl-to-tasks`, `ctl task create`)
go through normal governance. The map itself is a Markdown working artifact —
mutating it is a spec-tier write, recorded by the gate. Read the Frontier from
`ctl board` / `ctl next-task` rather than recomputing it by hand.
<!-- integration:opencode -->

The map lives at `.ctl/spec/maps/<slug>.md` (spec tier — writable). Mutating it
is gated by `.opencode/plugins/ctl-gate.ts`; protected paths remain hard-denied.
Seed from the confirmed alignment note; graduation via `ctl-to-tasks` /
`ctl task create` goes through normal governance. Read the Frontier from
`ctl board` / `ctl next-task`.
<!-- integration:claude -->

The map lives at `.ctl/spec/maps/<slug>.md` (spec tier — writable under the
gate). Seed from the confirmed alignment note. Run the per-session loop inline so
writes carry the active task's `CTL_TASK_ID` binding when one exists; read-only
Frontier checks (`ctl board`, `ctl next-task`) can be dispatched to a subagent
(built-in `Explore`, `claude-code-guide`). Graduation (`ctl-to-tasks`,
`ctl task create`) goes through normal governance.
