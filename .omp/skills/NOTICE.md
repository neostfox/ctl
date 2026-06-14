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
