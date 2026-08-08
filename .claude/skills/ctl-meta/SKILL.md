---
name: ctl-meta
description: "Understand and customize the ctl skill architecture. Use when modifying the skill surface (adding/editing a skill, changing the workflow, a managed core, or platform targeting), when onboarding to how ctl-the-binary relates to ctl-the-skills, or when the single-source generation / drift model needs explaining. Do NOT trigger for: routine implementation, governance (control-guard), or diagnosis (ctl-diagnose)."
---

# ctl-meta (Claude Code)

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
- **integration sections** — `

