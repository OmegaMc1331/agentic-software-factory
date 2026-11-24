# Conversation events and replay

P09 mission snapshots add bounded summaries of durable handoffs: identity, task, selected profiles, state and boundary. They exclude full patches and instruction payloads. The existing Rust observer persists snapshot changes under the same ordered conversation sequence. Replaying waiting/ready/executing/completed/rejected state does not submit another handoff or dispatch another task. Full history is available through the scoped handoff API. No fabricated token stream or frontend-only replacement status is added.

The existing authenticated local listener serves SSE at `GET /api/projects/:projectId/conversations/:conversationId/events`. No extra HTTP service or provider-token stream is introduced. Existing Host, Origin, session, cache and request protections apply.

Each normal SSE record has event name `conversation`, an `id` equal to the persisted event ID, and this JSON envelope:

```json
{
  "schema_version": 1,
  "id": 42,
  "conversation_id": "c_00000000000000000000000000000000",
  "event_type": "mission.updated",
  "payload": {"run_id": 1, "status": "active", "completed": 0, "total": 2},
  "timestamp": "2026-10-29T00:00:00+00:00"
}
```

The example illustrates the schema, not a real recorded mission. Actual evidence is retained with the delivery.

## Types

* `conversation.message.accepted`: durable user/response/command identities.
* `conversation.response.started`: validated processing began.
* `conversation.response.updated`: coordinator state changed.
* `conversation.response.completed`: final message persisted; this alone does not mean a linked workflow succeeded.
* `conversation.response.failed`: failure/interruption recorded.
* `mission.created`: transactional run association committed.
* `mission.updated`: a changed snapshot of actual persisted run, tasks, sessions and evidence.

There is no fabricated `response.delta`. Classification currently uses a final-structured CLI transport. Consumers reconcile full durable messages and mission state after event notifications. Technical stdout/stderr remain in existing agent-session views.

## Cursor and ordering

Use `after=<eventId>` on initial connection; standard `Last-Event-ID` is also accepted. IDs increase inside the project but need not be contiguous inside one conversation because other conversations use the same SQLite sequence. Events are selected by conversation and increasing ID, never by the browser's active project. Reads fetch at most 100 entries; queues are bounded separately.

The frontend store is keyed by project plus conversation. It ignores unknown schema versions, foreign conversation events and IDs at or below its last observed cursor. An event that arrives during an in-flight history read schedules another reconciliation, preventing a stale response from consuming the only update. Multiple Dockview panes share one store/subscription. Panel movement is not domain ownership.

## Reconnection and backpressure

The reader polls durable events at 500 ms intervals and sends keep-alives every 10 seconds. At most 32 conversation streams and 16 queued events per stream are allowed. Enqueue is nonblocking. A slow subscriber is disconnected when its queue is full; agent output, Runtime and other subscribers do not wait for it. The reconnecting client supplies its delivered cursor and also refreshes authoritative messages/missions. Replay may span several bounded connections; a snapshot refresh is the explicit resynchronization path.

Storage/replay failure emits `resync` when delivery capacity permits and closes the stream. The UI reports disconnection, refreshes its browser session, reconnects and offers durable-state reload. Malformed cursors are refused. A manually supplied future cursor does not authorize another resource; reload the view with its normal cursor if local state has been manually altered.

History remains in SQLite after browser or Rust restart. Runtime observers are independent of SSE lifetimes. Project reopening reconciles interrupted commands without repeating them. A browser disconnect has no P10 suspension semantics.

## References

The implementation follows the EventSource cursor/reconnection model in the [WHATWG HTML standard](https://html.spec.whatwg.org/multipage/server-sent-events.html). Browser behavior is tested against the installed Edge version; a web-tool retrieval of the standard timed out during this phase. Stable external-store snapshots follow [React's documented contract](https://react.dev/reference/react/useSyncExternalStore), retrieved 2026-10-29; the production project remains on its pinned React 18 dependency. Neither reference substitutes for the recorded local tests.

## P08 scheduling observations

The existing `mission.updated` snapshot now carries `active_tasks` (including a truthful review/wait/publication stage), `plan_revision`, historical revisions and integration records. Completed counts exclude Superseded work. The central conversation receives one concise runtime-sourced replan notice when an observed revision advances; repeated observation/replay does not duplicate it because snapshot comparison and message insertion share the immediate transaction.

Scheduler process events have their own durable schema-versioned project database IDs, queried through the scoped scheduler event cursor API. Concurrent producers are ordered by SQLite insertion; wall-clock completion order is not assumed to be deterministic. The P07 conversation stream remains its existing durable, bounded, authenticated, deduplicated replay channel. A displayed snapshot or a running task row alone does not prove process overlap; qualification uses supervised process IDs/lifetimes and OS observations.

Concurrent P08 observers compare plan revisions inside the short observation transaction. A delayed older snapshot cannot replace a newer revision or cause a duplicate replan notice when another browser reconnects. This monotonic revision guard does not introduce execution ownership into the browser.

## P10 lifecycle events

Migration 22 records mission lifecycle transitions in the existing scheduler event journal and mirrors bounded notifications into the P07 ordered conversation event stream. The outer event schema and project/conversation scope remain authoritative; payloads identify the run, lifecycle generation and applicable reason or checkpoint. The owning project database links the workspace identity. Events include `mission.suspend_requested`, `mission.quiescing`, `mission.checkpoint_created`, `mission.suspended`, `mission.resume_requested`, `mission.resumed`, `mission.recovery_required` and `mission.execution_settled`. The state model deliberately uses fewer durable states than the conceptual product sequence; checkpoint creation is a transaction/event, not a second independent scheduler.

Project-local workspace audit records `session.close_requested`, `session.closed`, `session.suspension_blocked` and `session.reopened`. Browser registration, last-seen timestamps and expiration are kept in the application presence audit, not high-frequency chat messages. Session closure is confirmed only after all affected lifecycle records have settled or suspended.

Concise conversation messages are emitted for confirmed suspension, resume, recovery requirement and terminal settlement. Heartbeats are never conversation messages. Existing sequence assignment, replay, deduplication and subscriber limits remain authoritative. A presentation storage quota cannot roll back a safety fence: the scheduler records `lifecycle_presentation_blocked` and the lifecycle REST view remains authoritative.


## P11 project memory integration

P11 does not emit heartbeat-style memory search traffic into chat. Major mission outcomes retain existing durable ordering. Memory references attach to the persisted conversation message provenance; extraction start/finish/failure uses the existing scheduler event journal. Memory usage is separately queryable by owning AgentSession, without asserting causal reliance.


## P12 review and Git evidence

P12 reads existing mission, task, AgentSession, review, handoff and integration identities rather than deriving authorship from commit text. Conversation/progress links continue to use the same project/run IDs. New human feedback and manual Git operation results are durable project-local records; they do not impersonate automated Reviewer messages or inject synthetic success events. Opening Source Control creates no model invocation, heartbeat chat traffic, memory extraction or publication.
