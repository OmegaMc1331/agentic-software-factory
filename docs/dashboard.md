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

### Agent Graph

The `Agent Graph` tab (`#/network`) renders the whole factory as a connected,
brain-like network rather than a single run. It is a separate React route in `App.tsx`,
read from the `GET /api/graph` endpoint (`fetchGraph` in `api.ts`).

- **Layout**: nodes are placed in left-to-right lanes — `agent`, `role`, `run`, `task` —
  with each lane spread across the full viewport height and deterministic jitter so the
  cloud reads as organic rather than gridded. Edges between lanes are curved beziers;
  same-lane `depends` edges (task → task) bow out as quadratics.
- **Node kinds**:
  - `agent` — blue when `available` (its command resolves on `PATH`), red when missing.
  - `role` — violet.
  - `run` — sky, with a status dot.
  - `task` — colored by `state`.
- **Edge kinds**: `binds` (role → agent), `uses` (run → planner role/agent),
  `contains` (run → task), `depends` (task → task).
- **Interaction**: hovering or clicking a node opens a side inspector (`NodeInfo.tsx`)
  with the node's real fields and — for tasks — its dependency and blocked relationships.
  The selected node, its neighbors, and connecting edges highlight while everything else
  dims.
- **Motion**: running runs and in-flight tasks get a subtle pulse ring; edges out of
  active runs (and into running tasks) animate a slow dash-flow. Both are suppressed
  under `prefers-reduced-motion`.
- The view is hand-rolled SVG/React (`NetworkGraph.tsx` + `NetworkView.tsx`); no graph
  library was introduced.

## Layout module

`src/layout.ts` is pure and unit-tested (Vitest):

- `computeLayout(tasks)` - assigns each task a level by longest dependency path, then
  x/y coordinates per level, and returns nodes, edges, and canvas size.
- `truncate(value, max)` - ellipsized labels for the graph nodes.

`src/networkLayout.ts` is also pure and unit-tested:

- `computeNetworkLayout(nodes, edges, opts)` - groups nodes by `kind` into lanes,
  positions each lane, and returns positioned nodes, edges (with per-kind control
  points), and canvas dimensions. Kept free of React imports so it can be unit-tested
  and reused.

## Checks

```bash
npm test            # vitest
npm run typecheck   # tsc --noEmit
npm run lint        # eslint
npm run format:check
npm run build       # vite build -> dist/
```

CI runs the first four against every push.