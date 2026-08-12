# Development

Contributor workflow for the Rust workspace and the dashboard.

## Rust

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Worktree tests (under `crates/factory-core/tests/e2e.rs` and
`crates/factory-git/src/lib.rs`) call real `git` and are intentionally slow. The CI
workflow runs the three commands above on every push.

## Dashboard

```bash
cd apps/dashboard
npm install
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
cd <project root> && target/debug/factory dev serve

# terminal 2:
cd apps/dashboard && npm run dev   # http://localhost:5173
```

The dashboard layout modules (`src/layout.ts`, `src/networkLayout.ts`) are pure,
unit-tested functions; keep them free of React imports.

## Building the dashboard into the served app

`factory start` serves `apps/dashboard/dist` when it exists at the working directory
or relative to the `factory` binary. Rebuild it after frontend changes:

```bash
cd apps/dashboard && npm run build
```

Restart `factory start` to pick up a newly built dashboard. `cargo build --workspace`
works without the dashboard built; the server then shows a page explaining how to build
it.

## CLI smoke test

```bash
cargo build
mkdir -p /tmp/factory-smoke && cd /tmp/factory-smoke
factory init
factory start
```