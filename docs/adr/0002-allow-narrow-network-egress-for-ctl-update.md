# ADR 0002 — Allow narrow network egress for `ctl update` (overturn DEP-002's no-HTTP-client stop)

- **Status:** Accepted
- **Priority:** Active — implemented by `ctl-update-self-update-v1`.
- **Date:** 2026-06-21
- **Affected guardrails:** `DEP-002` (STOP: "no async runtime / HTTP client / database / web framework"), the AGENTS.md "Standing hard stops" line ("No async runtime, HTTP client, … in the ctl core"), and the `check_dependencies` enforcement in `src/cli/mod.rs`.
- **Supersedes (in part):** the blanket no-HTTP-client reading that ADR 0001 relied on (ADR 0001 §Consequences cited "OIDC needs an HTTP client — forbidden by AGENTS.md"). That reasoning still holds for ADR 0001's scope (authenticated principals / OIDC); this ADR carves out **only** an operator-invoked self-update path and does not re-open crypto identity.

## Context

ctl ships through three channels: GitHub Releases (`neostfox/ctl`, via `scripts/install.{sh,ps1}`), npm (`@ai-dev/ctl`), and `cargo install`. Updating today is a manual, out-of-band step — the operator must re-run the installer or remember the right package-manager command. The request is a first-class `ctl update` that updates the binary in place to the latest release.

A self-updater must (1) discover the latest version, (2) fetch the new binary, and (3) replace the running one. Steps (1)–(2) require **outbound network I/O** — exactly what `DEP-002` and the AGENTS.md hard stops forbid in core.

Three designs were weighed (see the alignment exchange that produced this ADR):

1. **Delegating dispatcher** — shell out to the existing installer / package manager; no new dependency.
2. **Print-only advisor** — detect the install method and print the command; never execute.
3. **In-core downloader** — ctl itself performs HTTPS GET + sha256 verify + extract + self-replace.

The operator chose **(3), executing directly**, with `ureq` (synchronous — no async runtime) for HTTP and the **system `tar`** for extraction (present on Win10+/macOS/Linux), keeping the new *direct* dependency count at one. TLS uses the **`native-tls` backend** (OS-native: schannel on Windows, Security.framework on macOS, OpenSSL on Linux) rather than rustls — the local build environment (Windows) has no C compiler, and rustls' `ring`/`aws-lc-rs` providers require a C/asm toolchain; native-tls links the platform's existing TLS instead.

**Linux TLS amendment (build fix).** native-tls on Linux means `openssl-sys`, which by default *probes for a system OpenSSL install*. The release `aarch64-unknown-linux-gnu` artifact is **cross-compiled** and the runner has no ARM64 OpenSSL, so the first 0.0.6 build failed (`openssl-sys`: "Could not find directory of OpenSSL installation"). The fix is a Linux-only `ureq` `vendored` feature (forwards to `native-tls/vendored`): OpenSSL is **compiled from source** with the cross C toolchain the release workflow already installs (`gcc-aarch64-linux-gnu`). This keeps the `native-tls` decision, touches only the existing `ureq` dep (no new direct dependency — Windows/macOS are unaffected, still schannel/Security.framework), and statically self-contains every Linux artifact (no end-user libssl runtime dependency, which the prior system-linked build silently required).

## Decision

**Overturn the blanket no-HTTP-client stop with a narrow, explicit carve-out:** ctl core MAY perform outbound HTTPS **only** inside the `ctl update` command path, using a single synchronous client (`ureq`), against a **pinned release host** (`github.com` / `objects.githubusercontent.com` for `neostfox/ctl` release assets), and **only** to fetch a release whose artifact is then **sha256-verified** before it replaces the binary.

Everything else stays forbidden, and the guard is relaxed to match — not removed:

- `reqwest`, `tokio`, `hyper`, `async-std`, `actix-web` remain hard-blocked in `check_dependencies` (the `Cargo.lock` forbidden scan is unchanged). No async runtime enters core.
- The direct-dependency whitelist gains exactly `ureq` and nothing else.
- No inbound listener, no daemon, no database, no web framework — the README/DESIGN "NOT a daemon / web service / remote orchestration" identity is unchanged for the control loop.

## Rationale (why this carve-out is honest, not a slippery slope)

- **The network touches a maintenance command, not the control plane.** `ctl update` produces no canonical events, runs no reducer, and is never on the governed task/run/gate path. The event ledger stays pure, offline, and replayable — the property `DEP-002` actually protects (determinism + local-first governance) is untouched. The stop was over-broad for *self-maintenance*; it was right for *the runtime*.
- **It is operator-invoked and trusted-operator scoped.** Consistent with ADR 0001's threat model (local, single-user, trusts the operator). The operator already runs `curl … | sh` to install; `ctl update` is the same trust assumption with a sha256 check added, not a new attacker surface.
- **Integrity is preserved with an already-allowed primitive.** The downloaded artifact is verified against its published `.sha256` using the existing `sha2` dependency before any swap. A mismatch aborts without touching the installed binary.
- **Minimal blast radius by construction.** One sync client, no async runtime, extraction delegated to system `tar`, host pinned. The forbidden-dep scan still fails the build if anyone later reaches for `reqwest`/`tokio`.

## Consequences

**Accepted:**

- ctl core now links a TLS stack (ureq + native-tls → schannel / Security.framework / OpenSSL). Build now needs network access to crates.io. Linux builds use the **vendored** OpenSSL feature (compiled from source via the `ureq` `vendored` flag → `native-tls/vendored`), so they need a C compiler + `perl` at build time but **no** system OpenSSL and no runtime libssl; Windows/macOS use the OS TLS with no extra toolchain. The trade is a larger Linux binary and tracking OpenSSL CVEs at our release cadence, in exchange for a reliable cross build and a self-contained artifact.
- `check_dependencies` and the guardrail docs no longer read as an absolute "zero network ever"; they read as "no network except the audited `ctl update` egress." This ADR is the audit trail for that line.
- On Windows a running `.exe` cannot overwrite itself: `ctl update` renames the live binary aside (`ctl.exe` → `ctl.exe.old`) and writes the new one in place; the stale `.old` is best-effort removed on the next run. A failed swap leaves the original in place.

**Rejected alternatives:**

- *Delegating dispatcher / print-only* — lower footprint and no guardrail change, but the operator explicitly wanted a true in-place update, not a wrapper.

## Revisit triggers

Re-tighten or re-open if any of these change:

1. The egress stops being a single operator-invoked maintenance call (e.g. background polling, telemetry, an update daemon) — that would breach the "maintenance, not runtime" boundary this ADR rests on and must not happen under this ADR.
2. A second network use is proposed — it does **not** inherit this carve-out; it needs its own ADR.
3. The TLS dependency tree grows to pull a forbidden crate (async runtime) — the `Cargo.lock` forbidden scan should catch it; if it must be relaxed, stop and re-decide.

## Note — scope boundary, stated plainly

This ADR authorizes **outbound HTTPS for `ctl update` only**. It does **not** authorize: a general HTTP client elsewhere in core, any inbound network, any async runtime, OIDC/authenticated principals (still deferred by ADR 0001), or treating the network as part of the governed control loop.
