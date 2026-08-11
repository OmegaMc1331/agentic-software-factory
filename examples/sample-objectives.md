# Sample objectives

These objectives work with the local planner for offline demos. Each one is chosen to
produce a familiar dependency chain so the CLI and dashboard output is easy to read.

```bash
factory run "Build a small HTTP server in Rust that returns JSON"
factory run "Add a /health endpoint that responds with status JSON"
factory run "Port the CLI's --json flag and a unit test for the formatter"
factory run "Optimize the database index for the tasks table and verify with EXPLAIN"
```

Notes:

- Objectives are free-form; the plan structure is what matters. The local planner
  always returns a deterministic five-task pipeline, which is useful for demos and for
  reviewing state transitions without an API key.
- With a remote provider, write objectives that are concrete and bounded, and include
  the deliverable ("verify with EXPLAIN", "add a unit test") plus any constraints.