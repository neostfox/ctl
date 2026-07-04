---
name: ctl-brainstorm
description: "RETIRED — alias of ctl-grill-with-spec. The brainstorm interview (one question at a time, each with a recommended answer, evidence before questions) was absorbed into the grill v2 alignment station. Triggers when: /ctl-new is invoked or an old reference routes here. Always continue in ctl-grill-with-spec."
---

# ctl-brainstorm → ctl-grill-with-spec (alias)

This skill is retired. Its interview loop — evidence before questions, one
micro-decision at a time, every question carrying a recommended answer, converge
on a minimal task proposal — now lives in **`ctl-grill-with-spec`** (grill v2),
the single entry to the pipeline's alignment station:

```
triage (control-guard) → align (ctl-grill-with-spec) → PRD (ctl-to-prd)
      → tasks (ctl-to-tasks) → execute (ctl-tdd-loop) → wrap-up (ctl-spec-update)
```

Open `.omp/skills/ctl-grill-with-spec/SKILL.md` and continue there. The alignment
note goes to `.ctl/spec/alignment/<yyyy-mm-dd>-<slug>.md`; do not build until the
user confirms it.
