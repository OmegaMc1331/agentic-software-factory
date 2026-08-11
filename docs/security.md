# Security model

The factory is a local developer tool with no networked services of its own. Its
security surface is therefore small, but the boundaries matter.

## Trust boundaries

1. **Local filesystem.** `factory` writes only inside the factory root. The database is
   `.factory/db.sqlite3`; worktrees are `.factory/worktrees/`. No command accepts a
   path from the environment or an objective that influences where state is written.
2. **Git.** `factory-git` executes `git` invariantly as the invoking user. All
   arguments to the subprocess come from fixed command templates plus task ids and the
   detected repository paths; task titles and objectives never reach the command line.
3. **Model provider.** `factory run` sends the objective to the configured
   `FACTORY_BASE_URL`. The model response is parsed as strict JSON and validated, then
   discarded after planning. Only the derived plan and token usage are persisted.
4. **Local API.** `factory serve` binds `127.0.0.1` and exposes read-only endpoints.

## Credential handling

- API keys are read from `FACTORY_API_KEY` or `.env` at runtime and are never stored in
  the database or written to logs. `.env` and `.env.*` are ignored by git.
- `.env.example` documents variables with no secret values.
- Never commit a real key; the CI pipeline does not carry one.

## Database

- SQLite files are created within `.factory/` with the process default permissions.
- The schema has no extension points (no triggers, no attached databases, no
  user-supplied SQL). All statements are parameterized.

## The local API

The API surface is read-only and scoped to `127.0.0.1`:

- `GET /api/health`, `GET /api/runs`, `GET /api/runs/:id`
- CORS is permissive so the dashboard dev server (a separate origin on `:5173`) can
  proxy `/api`. Because every route is a GET that reads the shared database, permissive
  CORS cannot cause writes.
- The server does not expose the API key, `.env`, the provider configuration, or any
  filesystem path beyond the factory root.

## Known limitations

- No authentication on the local API. Do not expose port `4321` beyond a trusted
  machine.
- Secrets in `.env` are plaintext on disk, as is standard for local tools.
- The local planner never contacts the network; the remote provider is only contacted
  during `factory run` planning, with the objective as the request body.

## Reporting issues

Security concerns are handled like any other bug: open an issue with the affected
version, a minimal reproduction, and the expected versus actual behavior.