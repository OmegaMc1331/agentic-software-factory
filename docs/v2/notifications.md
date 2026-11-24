# Application notifications

Important project notifications are materialized in the owning project database. Migration 25 adds `application_notifications`; it does not replace conversation events, task evidence, AgentSessions or delivery journals.

## Actual event sources

SQL triggers record terminal/blocked run transitions, selected scheduler handoff/recovery/integration events, confirmed suspension/recovery-required transitions and persisted GitHub delivery outcomes in the same transaction as the source transition. Unique `source_event_id` prevents replay duplication. Heartbeats and routine token/task traffic are not notification sources.

Rust-owned service boundaries additionally record actual refused file saves, important requested IDE failures/approval denials and extension-installation failures. A closed enum chooses fixed text; browser requests cannot supply a title, error transcript or fake mission event. File-conflict provenance hashes the owned root/path/expected revision and deduplicates the same refused save. IDE/extension request failures are coalesced by kind in five-minute buckets to limit repeat noise. These service notices do not imply exhaustive detection of every asynchronous tool failure: the IDE's actual state/audit remains the detailed authority.

Messages contain generic recovery guidance and scoped identifiers, never objectives, source content, provider output, environment values or HTTP credentials. A notification cannot manufacture review approval, successful suspension, model availability or publication. Storage failure is logged and does not convert the original failed operation into success.

## Persistence and retention

Each record has an ID, unique source event, type, category, severity, fixed title/message, optional run/task, target, timestamps and revision. Important history survives restart. The latest 1,000 notifications per project are retained; source execution records have independent retention. Reads use bounded keyset pages (maximum 100). Read/dismiss state uses conditional revisions. An identical repeated desired state is idempotent; conflicting stale changes fail. Dismissal hides a retained record; users can include dismissed records and restore them. No route permits browser-created notifications.

Authenticated project GET/PATCH routes reuse Host/Origin/CSRF and runtime dispatch. An ID collision in another project does not confer access. The UI checks the response project before rendering its content/count. Notification links resolve their actual owning resource before navigation and acknowledgement. Missing/closed targets produce a recoverable error rather than opening a similarly named mission.

## Presentation and resources

The compact center supports unread/read, dismissal, project scope, event provenance, pagination and related-resource links. The top bar shows unknown when counts are unavailable. Serious outcomes remain in the center after a transient toast disappears.

One shared resource-cache subscription per open project polls at the existing ten-second cadence. Multiple mounted consumers share that request; unsubscribing releases its timer/abort owner. This adds no independent SSE stream. The first confirmed page initializes the arrival cursor, so reloading does not announce old records as new. New arrivals may produce one brief toast and, if explicitly enabled and granted, an OS notification. OS payloads contain only generic “ASF project event” guidance and an opaque deduplication tag. No private project name, objective or agent output is sent to the OS.

P10 remains manual-resume: a “resume available” notice opens the checkpoint/state and does not resume execution. GitHub notification status reflects persisted delivery state; merely viewing it never publishes.
