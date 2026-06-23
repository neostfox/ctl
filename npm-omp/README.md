# @velo-ai/omp

OMP plugin for the **ctl** control plane. It ships the governance pre-hook and the
control-plane skills, and depends on the `@ai-dev/ctl` npm package so the `ctl`
binary is installed alongside it.

> **Generated file — do not edit.** This package is produced from the canonical
> `.omp/` source by `ctl skills sync`. Edit `.omp/` (and the generator in
> `src/infrastructure/omp_plugin.rs`) instead; CI fails if `npm-omp/` drifts.

## Why a plugin

The hook (`hooks/pre/ctl-context.ts`) shells out to `ctl`. Resolving it by bare
name against the host process PATH fails on Windows when `ctl` was installed
somewhere off the launch PATH. Installing this plugin via npm places the platform
binary under `node_modules`, where the hook resolves it relative to the package —
no PATH dependence.

## Install

The extension hook only loads for **npm-installed** or **linked** plugins (not for
`omp plugin install github:…` marketplace installs).

```sh
# Local development against this repo's generated package:
omp plugin link ./npm-omp

# Or, once published:
npm i @velo-ai/omp
```

Override binary resolution with `CTL_BIN` if you want a specific `ctl`.
