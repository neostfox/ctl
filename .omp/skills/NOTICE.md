# Attribution

The control plane's skills adapt ideas and rubrics from the following open-source projects.
All are MIT-licensed; their concepts were re-expressed for the ctl/OMP workflow rather than
copied verbatim. Thanks to their authors.

| Project | Used for | License |
|---|---|---|
| [brooks-lint](https://github.com/hyhmrright/brooks-lint) | Decay-risk rubric (R1–R6 / T1–T6), the Iron Law finding schema, and the Health Score — see `ctl-review`, `.omp/spec/guides/{decay-risks,test-decay-risks,review-contract}.md` | MIT |
| [addyosmani/agent-skills](https://github.com/addyosmani/agent-skills) | SDLC skill shapes — interview/idea-refine/spec-driven (→ `ctl-brainstorm`), multi-axis code review and the mandatory Verification section (→ `ctl-review`) | MIT |
| [yao-bayesian-skill](https://github.com/yaojingang/yao-open-skills/tree/main/skills/yao-bayesian-skill) | Evidence grading (A–E), prior→posterior updating, and the disconfirming/falsification gate — see `ctl-diagnose`, `.omp/spec/guides/failure-diagnosis.md` | MIT |
| [pua](https://github.com/tanweai/pua) | Behavioral overlay for sub-agents — closure discipline ("where is the evidence?"), fact-driven attribution, and the exhaust-before-surrender mandate — see the dispatch constraints in `control-guard` and the closure checklist in `ctl-review` | MIT |

Earlier internal lineage: several skills descend from a prior **Trellis** workflow
(brainstorm, check, break-loop, spec bootstrap/update), now consolidated into the ctl/OMP
skill set above.

## Workflow skills foundation (L0 references)

The workflow skills — `ctl-grill-with-spec`, `ctl-to-prd`, `ctl-to-tasks`,
`ctl-tdd-loop`, `ctl-handoff`, and the canonical core at
`.agent/protocols/workflow-skills.md` — are **ctl-native rewrites** inspired by
Matt Pocock's engineering skill workflow and by Trellis PR #335:

| Source | Adapted into | Status |
|---|---|---|
| [mattpocock/skills](https://github.com/mattpocock/skills) — engineering skill workflow (setup → grill-with-docs → to-prd → to-issues → tdd → review → diagnose → improve-architecture → handoff) | the phase map and the grill / PRD / tasks / TDD / handoff skill shapes | L0 reference — not vendored |
| [mindfold-ai/Trellis PR #335](https://github.com/mindfold-ai/Trellis/pull/335) — First Principles / Bayesian thinking-framework placement | First Principles embedded in grill, Bayesian reasoning kept in `ctl-diagnose` (not floating "think better" skills) | L0 reference — not vendored |

These external materials are treated as **L0 reference material**: ctl adapts the
*ideas*, never vendors third-party skill text as an active control plane, and does
not place them inside its trust boundary. The skills are agent workflow
disciplines — they do not prove correctness and do not replace ctl gates, audits,
reviewer independence, or tamper evidence.
