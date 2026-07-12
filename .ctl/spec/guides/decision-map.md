# Decision Map (for efforts too big to plan upfront)

Read this guide when an effort is too large or foggy to resolve in a single
session — when `ctl-grill-with-spec` finds **fog**: decisions that cannot be
made until work on the frontier advances.

## When to build a map (and when not to)

Build a map **only** when grill surfaces fog. The test: are there decisions that
cannot be made until work on the frontier advances? If yes, the effort is too big
to plan upfront — build a map. If every decision resolves in the alignment
interview, the effort fits one session: go straight to `ctl-to-tasks` and skip
the map entirely.

**No-fog early exit.** A small or well-understood effort does not get a map. A
map for a no-fog effort is overhead that pays back nothing.

## What a map is — and is not

A decision map is a **human-maintained index**, not a store. A decision lives in
exactly one place — its ctl task once it graduates — so the map only gists and
links, never restates. It is not a machine projection (`control.json` already
gives the per-task machine view via `ctl board`); the map gives the **human
decision-arc** — what is resolved, what is still fog, what has been ruled out.

## The four sections

| Section | Captures |
|---|---|
| **Destination** | The outcome that must be true when done. One sentence. Every session re-orients here. |
| **Frontier** | ctl task IDs that are ready/active, with their `depends_on` satisfied. Each links to its task — the map does **not** restate objective or scope. Mirror what `ctl board` / `ctl next-task` compute; this section is for human orientation. |
| **Fog** | Decisions not yet resolvable because they depend on frontier work advancing. Each fog item names: the decision needed · what blocks it (which frontier task must complete) · what `kind` it graduates to (`implementation` / `research`) · **AFK / HITL**. |
| **Out of scope** | Ruled beyond the destination. Closed — never graduates. Distinct from fog: fog is in-scope-but-unresolved; out-of-scope is ruled out. |

## Graduation

When a fog item's blocker completes and the decision resolves, it **graduates**:

```
fog item → resolve the decision (HITL, via grill if needed)
         → ctl-to-tasks produces a slice (or ctl task create for a trivial one)
         → new task ID enters Frontier
         → fog item is struck from the map
```

Nothing lingers in two places. Graduating fog clears the patch so the decision
lives only in its task.

## The per-session loop

1. **Open the map** at `.ctl/spec/maps/<slug>.md`; re-read **Destination**.
2. **Check the Frontier** — `ctl board` / `ctl next-task` confirm which tasks are
   takeable. Claim or resume one.
3. **Work the task** under normal ctl governance (scope, gates, evidence).
4. **On discovery** — new decisions surface while working: add them to **Fog**
   with their blocker. Ruled-out work: add to **Out of scope**.
5. **On completion** — a frontier task done may unblock fog: resolve and graduate.
6. **Close** when Destination is reached and no fog remains.

## Where it lives

- `.ctl/spec/maps/<slug>.md` — spec tier, human-writable under the gate.
- Seed it from the alignment note (`ctl-grill-with-spec` output) when fog is found.
- It is **not** canonical truth and **not** a projection — it is a working
  artifact, like the alignment note, that the human maintains across sessions.

## Relationship to ctl's existing surface

The map does not duplicate ctl's mechanics — it orients a human across them:

- `depends_on` + `ctl next-task` → the Frontier (machine-computed; map mirrors).
- `ctl board` + `ctl drift` → per-task state and trouble (machine view).
- `ctl handoff` → compresses a session (the map survives across sessions).
- `ctl-to-tasks` → the graduation mechanism (fog → slice).

The map adds the one thing none of these carry: **what is still unresolved and
what has been ruled out**, at the project level.

## Provenance

Inspired by Matt Pocock's `wayfinder` skill (v1.1) — the fog-of-war / frontier /
"map is an index, not a store" framing — adapted to ctl's governed-task model.
External skill text is L0 reference material; this is a ctl-native rewrite, not
a vendored control.

## Template

```markdown
# Decision map: <slug>

status: draft · living · closed
destination: <one sentence — the outcome that must be true when done>

## Frontier
<!-- ctl task IDs ready/active; links only, no restatement -->
- [ ] <task-id> — link to `.ctl/tasks/<task-id>/`
- [x] <task-id> (done)

## Fog
<!-- in-scope decisions blocked on frontier work -->
- **<the decision>** · blocked by <task-id> · kind: implementation|research · HITL
- **<the decision>** · blocked by <task-id> · kind: research · AFK

## Out of scope
<!-- ruled beyond the destination; closed, never graduates -->
- <ruled-out work> — reason

## Log
- <yyyy-mm-dd> seeded from alignment note <slug>
- <yyyy-mm-dd> graduated fog "<decision>" → task <task-id>
- <yyyy-mm-dd> closed: Destination reached, no fog remains
```
