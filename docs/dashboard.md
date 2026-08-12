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
brain-like network rather than a single run: once the user's agents exist, it is
one workspace with a top toolbar, a large pannable/zoomable canvas, and a side
inspector. It is a separate React route in `App.tsx`, read from the
`GET /api/graph` endpoint (`fetchGraph` in `api.ts`), and polls every three seconds
while Live is on (toggle to pause).

![Agent network](assets/dashboard-network.png)

- **Layout**: a deterministic spring layout (`computeNetworkLayout`). Agents form a
  central hub on a loose ring; roles float in the orchestration band above them; runs
  sit below the hub; each run's tasks fan out underneath it, pulled into clusters by
  `contains` and `depends` edges. Everything is asymmetric — local density, jitter, and
  curved edges carry the neural feel without a rigid grid. All node boxes are resolved
  out of overlap and the canvas is sized to fit.
- **Node kinds**:
  - `agent` — a circular node with a status dot; blue outline when `available`, red when
    the command is missing. Shows the agent name, its assigned roles, and a small
    mono line when working (`working · #3`) derived from the active runs it pilots.
  - `role` — small violet pill.
  - `run` — pill with a status dot (sky when active).
  - `task` — pill colored by `state` (running/blocked get a stronger stroke, blocked is
    dashed); the `#id` is mono.
- **Edge kinds**: `binds` (role → agent), `uses` (run → planner role/agent),
  `contains` (run → task), `depends` (task → task). Edges are thin quadratics; an edge
  into a `blocked`/`failed` task is tinted, and edges into a `running` task animate a
  slow dash-flow (suppressed under `prefers-reduced-motion`).
- **Interaction**: pan by dragging, zoom with the wheel around the cursor, hover to
  focus, click to select. Selecting a node emphasizes it and its connecting edges,
  dims the rest without hiding context, and centers the view on it. Details (agent
  availability and activity, task dependencies, run counts) live in the side inspector.
- **Toolbar**: run selector (with multiple runs), Tasks/Dependencies toggles, Fit,
  Center, and Live/Paused. Without any runs the empty state shows the configured
  agent/role topology; with no agents configured it says so plainly.
- The view is hand-rolled SVG/React (`AgentGraph.tsx` + `GraphNode.tsx` +
  `GraphEdge.tsx` + `GraphToolbar.tsx` + `NodeInspector.tsx`, orchestrated by
  `NetworkView.tsx`); no graph library was introduced.

## Layout module

`src/layout.ts` is pure and unit-tested (Vitest):

- `computeLayout(tasks)` - assigns each task a level by longest dependency path, then
  x/y coordinates per level, and returns nodes, edges, and canvas size.
- `truncate(value, max)` - ellipsized labels for the graph nodes.

`src/networkLayout.ts` is also pure and unit-tested:

- `computeNetworkLayout(nodes, edges)` - assigns deterministic organic homes by kind
  (agent ring, role band, run band, per-run task fans), relaxes the graph over a fixed
  number of spring/link/gravity iterations, resolves label collisions, and returns
  positioned nodes, curved edge paths, and canvas dimensions. Kept free of React
  imports so it can be unit-tested and reused.

## Checks

```bash
npm test            # vitest
npm run typecheck   # tsc --noEmit
npm run lint        # eslint
npm run format:check
npm run build       # vite build -> dist/
```

CI runs the first four against every push.