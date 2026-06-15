# Divergence — U-1 doc correction (BS-U1)

> Originator artifact. L0 content. Task: `u1-doc-fix`.
>
> **No genuine divergence.** This task is an unambiguous documentation
> correction whose content is fully determined by already-recorded evidence:
> uncertainty `U-1` on task `uncertainty-ledger-v1` (status `open`), which
> recorded that `EPISTEMIC_CONTROL.md` §4 cites `.ctl/brainstorms/<id>.json` as
> the L0 artifact path while the enforced boundary protects the entire `.ctl/`
> tree, so brainstorm artifacts must live in the tracked top-level `brainstorms/`
> directory.
>
> There are no competing directions to explore: the correct path is dictated by
> what the `PathNormalizer` actually enforces. This artifact exists only to
> satisfy the originator-reference requirement of the skip path; the critic step
> is skipped with reason `unambiguous_existing_spec` (see the recorded
> `brainstorm_skipped` disposition).
>
> Scope: fix the path examples in §4 (and the §6/§4 "under `.ctl/`" example) and
> add the `.ctl/` (protected control state) vs `brainstorms/` (tracked L0
> cognitive artifacts) distinction. Nothing else; U-2 is explicitly out of scope.
