# Development

Contributor workflow for the Rust workspace and the dashboard. Normal users should use
the one-command installers in the [README](../README.md) and never build from source.

## Prerequisites

- Rust (stable) with `cargo`
- Node.js 20+ and npm, only for the dashboard

## Rust workspace

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Worktree tests (under `crates/factory-core/tests/e2e.rs` and `crates/factory-git`) call
real `git` and are intentionally slow.

## Dashboard

```bash
cd apps/dashboard
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
```

For frontend development use the Vite dev server, which proxies `/api` to the factory
API:

```bash
# terminal 1: run the local API on 4321
cd <project root> && cargo run -p factory-cli -- dev serve

# terminal 2:
cd apps/dashboard && npm run dev   # http://localhost:5173
```

The dashboard layout modules (`src/layout.ts`, `src/networkLayout.ts`) are pure,
unit-tested functions; keep them free of React imports.

## The dashboard in the binary

In release builds the dashboard is embedded into the binary, so copying `factory` (or
`factory.exe`) alone is a complete installation. The embedding is driven by the
`embedded-dashboard` cargo feature on `factory-cli`/`factory-api` (using
[rust-embed](https://github.com/pyrossh/rust-embed)). The feature requires
`apps/dashboard/dist` to exist at compile time.

```bash
cd apps/dashboard && npm ci && npm run build
cd ../..
cargo build --release --features embedded-dashboard -p factory-cli
```

The release build fails at compile time if the dashboard has not been built. Plain
`cargo build` (debug or release, no feature) still compiles without the dashboard; the
server then serves `apps/dashboard/dist` from disk when present, or a stub page that
explains how to build it.

The embedded dashboard is covered by tests in `crates/factory-api/tests/api.rs`:

```bash
cargo test --release --features embedded-dashboard -p factory-api
```

## CLI smoke test

```bash
cargo build --release --features embedded-dashboard -p factory-cli
mkdir -p /tmp/factory-smoke && cd /tmp/factory-smoke
/home/you/path/to/factory init
/home/you/path/to/factory start
```

## Releasing

Tagging a version builds and publishes prebuilt binaries via
[`.github/workflows/release.yml`](../.github/workflows/release.yml):

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow builds the dashboard, compiles the release binary with the
dashboard embedded for Windows x86_64, Linux x86_64, macOS Apple Silicon, and macOS
Intel, packages `factory` + `LICENSE` per platform with SHA-256 checksums, verifies the
artifacts, and creates a GitHub Release. Tags containing a dash (`v0.2.0-rc.1`) are
published as prereleases. Installers `install.ps1` and `install.sh` at the repository
root are exercised against those archives by the CI workflow's `installers` job.

## CI

`.github/workflows/ci.yml` runs on every push and pull request:

- Rust checks on `ubuntu-latest` and `windows-latest` (fmt, clippy with
  `-D warnings`, full test suite)
- Dashboard checks (format, lint, typecheck, tests, build)
- A release-style build with the embedded dashboard plus a smoke test of a standalone
  copy of the binary outside the repository
- An end-to-end installer test (both installers against locally packaged archives)