# @velo-ai/omp

OMP plugin for the **ctl** control plane: the governance pre-hook and the
control-plane skills. Pure integration package — it does **not** bundle the
`ctl` binary.

> **Generated file — do not edit.** This package is produced from the canonical
> `.omp/` source by `ctl skills sync`. Edit `.omp/` (and the generator in
> `src/infrastructure/omp_plugin.rs`) instead; CI fails if `npm-omp/` drifts.

## Install

1. Install `ctl` itself: `cargo install --path .` from the ctl repo, or download
   a binary from the GitHub releases page. The hook resolves it via the one
   blessed chain **CTL_BIN → `~/.cargo/bin` → PATH** — set `CTL_BIN` to pin a
   specific binary.
2. Install the plugin. The extension hook only loads for **npm-installed** or
   **linked** plugins (not for `omp plugin install github:…` marketplace
   installs):

```sh
# Local development against this repo's generated package:
omp plugin link ./npm-omp

# Or, once published:
npm i @velo-ai/omp
```
