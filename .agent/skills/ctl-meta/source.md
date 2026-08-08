---
name: ctl-meta
description: "Understand and customize the ctl skill architecture. Use when modifying the skill surface (adding/editing a skill, changing the workflow, a managed core, or platform targeting), when onboarding to how ctl-the-binary relates to ctl-the-skills, or when the single-source generation / drift model needs explaining. Do NOT trigger for: routine implementation, governance (control-guard), or diagnosis (ctl-diagnose)."
---

# ctl-meta

The self-documentation skill for the ctl skill **surface** — how the skills are
built, sourced, and kept in sync, so you can modify them correctly.

ctl is a **Rust control-plane binary** (facts, scope, gates, ledger) plus a
**skill layer** (semantic workflow). The non-negotiable separation between them
— skills manage workflow and never relax a boundary, never declare a task
complete, never substitute their judgement for ctl evidence; workflow discipline
is not proof — is stated **once**, canonically, in
`.agent/protocols/workflow-skills.md` (division of labor) and `AGENTS.md`
(architecture + dependency direction). This skill does **not** restate it; it
covers only the skill-surface mechanics below.

## How the skill surface is built (single-source generation)

Every **generated** skill has ONE canonical source at `.agent/skills/<name>/source.md`.
From it, `ctl skills sync` generates each platform's `SKILL.md`
(`.omp/skills/`, `.claude/skills/`, `.opencode/skills/`). The generated files
are committed; the CI gate `ctl skills sync --check` fails if anyone hand-edits
one without re-syncing.

```
.agent/skills/<name>/source.md   ← the ONLY place to edit a skill
        │  (frontmatter + optional phase body + per-platform integration sections)
        ▼  ctl skills sync
.omp/skills/<name>/SKILL.md
.claude/skills/<name>/SKILL.md
.opencode/skills/<name>/SKILL.md
```

**Not every skill is generated.** ctl-review and ctl-diagnose are
single-platform (OMP), core-less skills with no cross-platform divergence —
generating them would be pure indirection (the source would just be the skill
with a generated H1). So they are hand-authored directly at
`.omp/skills/<name>/SKILL.md` and ship from there. **The rule: generation pays
for itself only where a managed core must stay in sync across copies or where
platforms diverge.**

Optional **progressive-disclosure references** live at
`.agent/skills/<name>/references/*.md`; `ctl skills sync` copies them verbatim
into each platform skill dir and `ctl init` ships them. They hold supplementary
depth (e.g. `control-guard/references/command-reference.md`) that the skill
body links and an agent loads on demand — a context-economy optimization for
current models. The skill stays correct without them, and the whole layer is
designed to be dropped in one pass when models no longer need it (search for the
`Progressive-disclosure references` embed blocks).

A `source.md` carries:
- **frontmatter** — shared `name` + `description` (the trigger contract).
- **phase body** — the shared, platform-neutral instructions (empty for
  control-guard; the whole skill for plain skills).
- **integration sections** — `<!-- integration:<platform> -->` blocks carrying
  platform-specific mechanics (hooks, subagent rosters). Plain skills have none.

## Managed cores and the drift contract

Two skills embed a **managed core** — a byte-identical protocol block wrapped in
markers, sourced from a canonical file under `.agent/protocols/`:

| Skill | Family | Canonical core | Reference part? |
|---|---|---|---|
| control-guard | control-guard | `.agent/protocols/control-guard.md` | no |
| ctl-grill-with-spec / ctl-to-prd / ctl-to-tasks | workflow | `.agent/protocols/workflow-skills.md` | yes (phase map after the marker) |

The other skills (ctl-spec, ctl-cognitive, ctl-review, ctl-diagnose, ctl-meta)
are **plain** — no managed core; the source body is the whole skill.

Two independent drift tests (`control_guard_protocol_sync`,
`workflow_protocol_sync` in `src/infrastructure/skills.rs`) refuse to let an
embedded core diverge from its canonical source. They reuse the exact
parse/normalize/compare primitives that `ctl adapter doctor` runs at runtime.
**Never hand-edit a managed core inside a platform SKILL.md** — edit the
canonical protocol, then `ctl skills sync`.

## Platform targeting (intentional asymmetry)

Each skill declares which platforms it ships to. The asymmetry is deliberate —
different platforms have different mechanisms:

| Skill | OMP | Claude | OpenCode |
|---|---|---|---|
| control-guard | ✓ | ✓ | ✓ |
| ctl-grill-with-spec / ctl-to-prd / ctl-to-tasks | ✓ | ✓ | ✓ |
| ctl-spec | ✓ | ✓ | ✓ |
| ctl-cognitive | ✓ | ✓ | — |
| ctl-review | ✓ | — | — |
| ctl-diagnose | ✓ | — | — |
| ctl-meta | ✓ | ✓ | ✓ |

ctl-review/ctl-diagnose are OMP-centric (`reviewer`/`scout` rosters); on Claude
review runs via the control-guard review gates + `ctl-oracle`, on OpenCode via
`explore`/`oracle` roles. ctl-cognitive omits OpenCode. Adding or changing a
platform target means editing the skill's `platforms` row in
`src/infrastructure/skill_sync.rs`.

## How to modify the skill surface

- **Edit a skill's content** → edit `.agent/skills/<name>/source.md`, run
  `ctl skills sync`, commit the regenerated files.
- **Edit a managed-core protocol** → edit `.agent/protocols/<family>.md`, run
  `ctl skills sync` (re-embeds the core into every platform skill), commit. The
  drift test confirms every copy re-matches.
- **Add a skill** → create `.agent/skills/<name>/source.md`, add a `SkillSpec`
  row in `src/infrastructure/skill_sync.rs` (name, title, family, platforms), run
  `ctl skills sync`. If it must ship via `ctl init`, also add it to the relevant
  embed list in `src/infrastructure/skills.rs`.
- **Add a new managed core** → add a `Family` (canonical path + markers + version
  decl) and a canonical protocol under `.agent/protocols/`, then a drift test in
  `skills.rs` mirroring the two existing ones.

## Do not

- Do not hand-edit a generated `SKILL.md` — it is regenerated from source.
- Do not edit a managed core outside `.agent/protocols/`.
- Do not vendor third-party skill trees as an active control (the provenance test
  forbids it); external ideas are adapted, never imported verbatim.
- Do not put project-specific rules into a ctl skill — those belong in
  `.ctl/spec/` or `.ctl/config.toml`.

## Where things live (quick reference)

| Path | Role |
|---|---|
| `.agent/skills/<name>/source.md` | canonical source for every skill |
| `.agent/protocols/control-guard.md` | control-guard managed-core source |
| `.agent/protocols/workflow-skills.md` | workflow managed-core source (+ reference part) |
| `src/infrastructure/skill_sync.rs` | the generator: families, skill registry, `sync`/`compose` |
| `src/infrastructure/skills.rs` | embed lists (`ctl init` shipping) + drift tests |
| `.omp/.claude/.opencode skills/<name>/SKILL.md` | generated output (committed) |
