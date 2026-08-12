# Dashboard

The dashboard is a read-only developer-tool interface over the factory API. It shows
what the factory knows: runs, task states, the planner agent, and the dependency graph.
All state changes must still go through the CLI; the dashboard never mutates anything.

## Stack

- Vite + React 18 + TypeScript
- Vitest for the graph layout module and utilities
- ESLint (flat config) + Prettier

## Running

```bash
# terminal 1: the factory API
factory serve

# terminal 2:
cd apps/dashboard
npm install
npm run dev
```

`vite.config.ts` proxies `/api` to `http://127.0.0.1:4321`, so the app talks to the
same origin during development and matches the production build's `/api` prefix.

## Views

### Runs table

The main view lists every run: id, objective, status, planner agent, a progress bar
(completed/total), and creation time. Clicking a row opens the run detail.

### Run detail

- Run header: id, status, completion percentage, objective.
- Meta grid: planner agent, created time.
- **Task graph**: an SVG DAG rendered from actual dependency rows. Tasks are layered by
  longest path from the roots; independent tasks share a level, so the graph reads
  left-to-right like a CI pipeline.
- **Tasks table**: id, title, objective, status badge, dependency list, and worktree
  path.

## Layout module

`src/layout.ts` is pure and unit-tested (Vitest):

- `computeLayout(tasks)` - assigns each task a level by longest dependency path, then
  x/y coordinates per level, and returns nodes, edges, and canvas size.
- `truncate(value, max)` - ellipsized labels for the graph nodes.

## Checks

```bash
npm test            # vitest
npm run typecheck   # tsc --noEmit
npm run lint        # eslint
npm run format:check
npm run build       # vite build -> dist/
```

CI runs the first four against every push.