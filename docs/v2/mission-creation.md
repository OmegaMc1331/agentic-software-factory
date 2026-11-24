# Mission creation through the conversation

All three P07 entry methods submit the same project/conversation command endpoint. No competing workflow or scheduler is created.

## Chat

The default central Orchestrator view persists a user message and invokes the configured Planner agent for a finite action. A valid `create_mission` action creates one real run and automatically invokes Factory's Planner, validates the plan and calls the existing start/policy path. Normal authorized missions require no additional plan-approval click. Questions, clarification and unavailable capabilities need not create a workflow.

## Form

Mission form sends an explicit `mission` contract alongside the originating message. It contains objective, scope guidance, constraints, acceptance criteria, optional existing recipe and optional existing team selection. There are no decorative autonomy, resource or publication controls that the current runtime cannot enforce. The existing role assignments and recipe/policy snapshot remain authoritative.

Scope, constraints and acceptance fields become bounded planning input and are retained in command provenance. They are not a new filesystem permission system. External publication remains an independently authorized existing delivery operation; P07 never calls push/PR publication automatically.

## Visual plan

The visual editor uses the existing React Flow dependency, backed directly by the submitted `Plan` data. Users create/edit tasks, objectives, dependencies, criteria, roles and compatible operations. Node selection and dragging complement an ordinary keyboard-operable task list. Dependencies can be edited without dragging.

Client validation identifies missing tasks, duplicate IDs, cycles and missing criteria. Rust repeats DAG validation and then uses Factory's allowed-team-role and role/operation normalization. The authored graph is persisted through the same task/plan-revision infrastructure. It does not invoke a Planner to silently replace the user's plan. Additional form context remains source metadata; the executable authored Plan is the user's submitted contract.

Submission is the explicit launch decision. After validation, it starts without a redundant second confirmation. An invalid graph cannot execute. A valid graph containing an unavailable/incompatible role creates a recorded failed or blocked attempt at launch and never remains falsely labelled as active execution.

## Durable idempotency

Every submission includes a client-generated key. The server hashes the complete request payload and enforces unique `(conversation_id, command_key)` in SQLite. Repeating the same key and payload returns its original command; changing the payload with that key returns a conflict. Identical text with a deliberately new key is a new user command.

Message/command acceptance is transactional. Run creation and the command-to-run link are another single transaction after configuration validation. Browser retries never directly call `create_workflow` again. A missing acknowledgment is reconciled using the retained key. After restart, uncertain unfinished work is interrupted and inspected, not automatically reissued.

## Execution and outcomes

The existing Runtime reserves one owned run operation for planning, automatic start and execution. It retains its cancellation flag across the handoff and transfers the already-held application permit. Factory's `prepare_start` still checks DAGs, assigned agents, roles, policies and Git state. The P07 layer neither schedules parallel tasks nor replans an active DAG.

Progress displays actual run states (`planning`, `planned`, `active`, `blocked`, `completed`, `failed`, `cancelled`) and completed/total task counts. It never invents a percentage. Current tasks, actual sessions, attempts, commands, artifacts, changed-file evidence and integration SHA are linked to the original V1-compatible workflow details.

Explicit cancellation targets the owning project's command and existing Runtime operation. A pre-planning cancellation is retained; a running cancellation uses the established process-tree path. Closing/hiding a view or switching a project does not cancel or suspend work.

## Partial failures

| Failure | Durable outcome |
| --- | --- |
| Message accepted; agent/config/output fails | User message and failed response remain; no unvalidated action executes |
| Mission resolution fails before creation | Command failure with no fabricated run |
| Run created; Planner fails | Associated failed run, rejected sessions and original objective retained |
| Authored role/operation invalid | No task execution; unplanned run is failed rather than left falsely planning |
| Valid plan; policy/start blocks | Existing blocked/planned run and actionable workflow evidence remain |
| Browser disconnects after start | Runtime continues; cursor/history reconciliation restores presentation |
| Run finishes; result persistence fails | Actual run outcome remains independent; response failure never rewrites run success |
| Server stops during an interaction | Command becomes interrupted on project reopen; no automatic side-effect retry |

P08 scheduling/replanning, P09 agent transfer/resource policies and P10 pause/checkpoints are not implemented by these flows.
