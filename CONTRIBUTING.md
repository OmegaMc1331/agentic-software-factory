# Contributing

Thanks for your interest. This project is intentionally small and opinionated; keep
changes aligned with its core principles.

## Principles

1. **The system owns state.** Never scatter state outside `.factory/`. The main working
   tree stays clean.
2. **No comments in source code.** Code must read clearly on its own. If a block needs
   prose to be understood, extract it into a named function or a doc rendered elsewhere.
3. **No filler.** No unused functions, no placeholder exports, no boilerplate copied
   from a template "because it is conventional".
4. **Verifiable by construction.** Every task, command, and endpoint has tests. Rule of
   thumb: a change that cannot be demonstrated by a test has not landed.
5. **Strict transitions.** Do not loosen the task state machine to paper over a bug.
   If a transition looks wrong, the caller is wrong.

## Development setup

See [docs/development.md](docs/development.md) for the full contributor workflow.

```bash
cargo build
cargo test
cd apps/dashboard && npm install
```

To build a release binary with the dashboard embedded (as shipped), build the dashboard
first, then `cargo build --release --features embedded-dashboard -p factory-cli`.

## Making changes

- Run `cargo fmt --all` and `cargo clippy --workspace --all-targets` before committing;
  CI fails on violations.
- For the dashboard run `npm run format`, `npm test`, `npm run typecheck`, and
  `npm run lint`.
- Add or update tests for every behavior change. Worktree tests under
  `crates/factory-core/tests/` (e2e) and `crates/factory-git/` are intentionally slow
  because they call git.

## Commit messages

Concise, imperative mood, focused on one logical change (for example:
`factory-core: reject completed without worktree`, or `dashboard: render dependency
edges`). Prefer several small commits over one large one.

## Pull requests

Describe what changed and why, and point at the tests that prove it. No generated
boilerplate, no screenshots of mock data.

## License

By contributing you agree that your contributions are licensed under the MIT license;
see [LICENSE](LICENSE).