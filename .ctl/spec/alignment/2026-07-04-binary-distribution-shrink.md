# Alignment: shrink binary distribution (B-lite)

status: confirmed (2026-07-04, user)
station: align (ctl-grill-with-spec)
downstream: ctl-to-prd → ctl-to-tasks

## Observed facts (cited)

- The binary-pit inventory (session 2026-07-04) found 16 pits; the
  distribution/resolution class (#1-#4, #13, #14) traces to one root: the
  governance semantics live in a compiled binary copied to N locations and
  chosen by 4 independent resolvers (ctl-gate.py full chain; ctl-context.py
  and ctl-wrapup.py bare `"ctl"`; opencode ctl-gate.ts its own TS chain).
- The npm surface is 5 platform packages + `@velo-ai/ctl` meta + `@velo-ai/omp`
  (bundles the binary as a dependency), plus the Windows-shim workaround that
  motivated the resolver complexity in the first place
  (claude-gate-ctl-resolve-v1 lineage).
- External npm users ≈ 0 at 0.0.x; the repo's own architecture-review skill
  lists "hypothetical (not real) adapter boundaries" as a structural smell.
- Global-npm shadowing bit twice in one day (stale 0.0.9 exe shadowing fresh
  cargo-installed 0.0.10); the workaround is a manual Copy-Item.

## Confirmed decisions (user, 2026-07-04)

1. **Retire npm binary distribution**: stop publishing the 5 platform packages
   and the `@velo-ai/ctl` meta package; `@velo-ai/omp` becomes a pure
   hooks+skills package (no bundled binary). Existing published versions stay
   on the registry (deprecation notice optional, decide at release time).
   History preserved in git; revive if real npm demand appears.
2. **One resolver chain everywhere**: `CTL_BIN → ~/.cargo/bin → PATH`. Delete
   local/global npm probing from ctl-gate.py, opencode ctl-gate.ts, and the
   omp hook; ctl-context.py and ctl-wrapup.py adopt the SAME resolver (today
   they use bare "ctl" — pit #2).
3. **Version visibility**: gate/context verdicts carry `"version"`
   (CARGO_PKG_VERSION); SessionStart context shows it; `ctl adapter doctor`
   gains a skew check enumerating resolver candidates and comparing
   `--version` (also checks python availability — pit #16).
4. Install story: `cargo install --path .` (dev) or GitHub release download
   (users). `ctl update` self-updater remains the single in-band updater.

## Non-goals

- No interpreted rewrite (option C) — a spike may be evaluated later; today's
  pain does not pay for a full rewrite.
- No npm unpublish of existing versions (destructive, registry-limited).
- SAC/WDAC (pit #5) is NOT addressed by this — it is a machine-policy
  decision, currently wait-and-see.

## Consequences / risks

- Reverses parts of the claude-gate-ctl-resolve-v1 / omp-hook-global-npm-resolve
  lineage — needs an ADR naming what those tasks solved (Windows npm shims)
  and why the premise (npm distribution) is being removed instead.
- `.ctl/gate-binary-resolution` memory notes and AGENTS.md/skills text
  referencing npm resolution become stale — update in the same batch.
- release.yml sheds the npm publish steps; the OIDC provenance fragility
  (0.0.9 incident class) disappears with them.

## Execution queue (after SAC unblocks compilation)

1. Finish the stuck `wrapup-stop-hook-v1` (rerun 4 gates → audit → finish →
   reinstall cargo bin; npm-global copy becomes moot after this batch).
2. `memory-two-tier-v1` (PRD wrapup-memory-capture task 2/3).
3. **`binary-dist-shrink-v1`** (this note): resolvers + version handshake +
   doctor skew/python check + npm retirement + ADR + docs/memory sync.
