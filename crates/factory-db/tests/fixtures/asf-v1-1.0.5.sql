BEGIN TRANSACTION;
CREATE TABLE agent_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    agent TEXT NOT NULL,
    command TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    exit_code INTEGER,
    duration_ms INTEGER,
    stdout TEXT,
    stderr TEXT
, attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL, mode TEXT NOT NULL DEFAULT 'automated', operation TEXT, policy_audit TEXT, output_meta TEXT);
INSERT INTO "agent_sessions" VALUES(1,1,NULL,'planner','fixture-agent','SYNTHETIC fixture command - never executed','success','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z',0,10,'SYNTHETIC retained output','',NULL,'automated','planning','{"source":"role:worker","filesystem":"restricted","network":"deny","environment":"filtered","writeScopes":["src/**"],"toolAllow":["fixture.inspect"],"toolDeny":[]}','{"limitBytes":1048576,"stdoutBytes":20,"stderrBytes":0,"stdoutDroppedBytes":0,"stderrDroppedBytes":0,"stdoutTruncated":false,"stderrTruncated":false}');
INSERT INTO "agent_sessions" VALUES(2,1,1,'architect','fixture-agent','SYNTHETIC fixture command - never executed','success','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z',0,10,'SYNTHETIC retained output','',1,'automated','advisory','{"source":"role:worker","filesystem":"restricted","network":"deny","environment":"filtered","writeScopes":["src/**"],"toolAllow":["fixture.inspect"],"toolDeny":[]}','{"limitBytes":1048576,"stdoutBytes":20,"stderrBytes":0,"stdoutDroppedBytes":0,"stderrDroppedBytes":0,"stdoutTruncated":false,"stderrTruncated":false}');
INSERT INTO "agent_sessions" VALUES(3,1,2,'worker','fixture-agent','SYNTHETIC fixture command - never executed','success','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z',0,10,'SYNTHETIC retained output','',2,'automated','implement','{"source":"role:worker","filesystem":"restricted","network":"deny","environment":"filtered","writeScopes":["src/**"],"toolAllow":["fixture.inspect"],"toolDeny":[]}','{"limitBytes":1048576,"stdoutBytes":20,"stderrBytes":0,"stdoutDroppedBytes":0,"stderrDroppedBytes":0,"stdoutTruncated":false,"stderrTruncated":false}');
INSERT INTO "agent_sessions" VALUES(4,1,2,'reviewer','fixture-agent','SYNTHETIC fixture command - never executed','success','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z',0,10,'SYNTHETIC retained output','',2,'automated','review','{"source":"role:worker","filesystem":"restricted","network":"deny","environment":"filtered","writeScopes":["src/**"],"toolAllow":["fixture.inspect"],"toolDeny":[]}','{"limitBytes":1048576,"stdoutBytes":20,"stderrBytes":0,"stdoutDroppedBytes":0,"stderrDroppedBytes":0,"stdoutTruncated":false,"stderrTruncated":false}');
INSERT INTO "agent_sessions" VALUES(5,NULL,NULL,'worker','fixture-agent','SYNTHETIC fixture command - never executed','success','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z',0,10,'SYNTHETIC retained output','',NULL,'interactive',NULL,NULL,'{"limitBytes":1048576,"stdoutBytes":20,"stderrBytes":0,"stdoutDroppedBytes":0,"stderrDroppedBytes":0,"stdoutTruncated":false,"stderrTruncated":false}');
CREATE TABLE github_deliveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'not_ready',
    repository TEXT,
    remote TEXT,
    base_branch TEXT,
    head_branch TEXT NOT NULL,
    pushed_head TEXT,
    pr_number INTEGER,
    pr_url TEXT,
    pr_state TEXT,
    pr_is_draft INTEGER,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, operation_id TEXT, recovery_required INTEGER NOT NULL DEFAULT 0);
INSERT INTO "github_deliveries" VALUES(1,1,'interrupted','fixture/offline','origin','main','factory/run-1','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',NULL,NULL,NULL,NULL,'SYNTHETIC uncertain external outcome - must not replay','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','synthetic-v1-operation',1);
CREATE TABLE github_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    repository TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    issue_url TEXT,
    issue_title TEXT NOT NULL,
    issue_body TEXT NOT NULL,
    issue_state TEXT,
    issue_author TEXT,
    issue_labels TEXT NOT NULL DEFAULT '[]',
    issue_comments TEXT NOT NULL DEFAULT '[]',
    imported_at TEXT NOT NULL
);
INSERT INTO "github_links" VALUES(1,1,'github','fixture/offline',42,'https://github.com/fixture/offline/issues/42','SYNTHETIC issue - never fetched','SYNTHETIC untrusted external data','open','fixture-user','["fixture"]','[]','2026-10-26T00:00:00Z');
CREATE TABLE integration_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER NOT NULL REFERENCES task_attempts(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    outcome TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO "integration_outcomes" VALUES(1,1,2,2,'fixture-agent','clean','2026-10-26T00:00:00Z');
CREATE TABLE mcp_server_leases (
    server_id TEXT PRIMARY KEY,
    active_calls INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
INSERT INTO "mcp_server_leases" VALUES('fixture',0,'2026-10-26T00:00:00Z');
CREATE TABLE mcp_tools (
    server_id TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    description TEXT NOT NULL,
    input_schema TEXT NOT NULL,
    annotations TEXT,
    classification TEXT NOT NULL,
    discovered_at TEXT NOT NULL,
    PRIMARY KEY (server_id, tool_name)
);
INSERT INTO "mcp_tools" VALUES('fixture','inspect','SYNTHETIC offline tool','{"type":"object","properties":{}}','{"readOnlyHint":true}','read','2026-10-26T00:00:00Z');
CREATE TABLE plan_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    source TEXT NOT NULL,
    reason TEXT,
    planner_session_id INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
    snapshot TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(run_id, revision)
);
INSERT INTO "plan_revisions" VALUES(1,1,1,'planner','SYNTHETIC original plan',1,'{"objective":"SYNTHETIC V1 preservation workflow","tasks":[{"id":1,"runId":1,"title":"Architecture","objective":"SYNTHETIC fixture task","acceptanceCriteria":["SYNTHETIC preservation criterion"],"state":"completed","position":0,"worktreePath":".factory/worktrees/t1","createdAt":"2026-10-26T00:00:00Z","updatedAt":"2026-10-26T00:00:00Z","role":"architect","operation":"advisory","dependencies":[],"agentOverride":null},{"id":2,"runId":1,"title":"Implementation","objective":"SYNTHETIC fixture task","acceptanceCriteria":["SYNTHETIC preservation criterion"],"state":"completed","position":1,"worktreePath":".factory/worktrees/t2","createdAt":"2026-10-26T00:00:00Z","updatedAt":"2026-10-26T00:00:00Z","role":"worker","operation":"implement","dependencies":[1],"agentOverride":"fixture-agent"}]}','2026-10-26T00:00:00Z');
CREATE TABLE role_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    role TEXT NOT NULL,
    operation TEXT,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO "role_artifacts" VALUES(1,1,1,1,'architect','advisory','architecture','{"summary":"SYNTHETIC architecture decision","findings":["Preserve V1 records"]}','2026-10-26T00:00:00Z');
CREATE TABLE routing_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    mode TEXT NOT NULL,
    selected_agent TEXT NOT NULL,
    role TEXT,
    operation TEXT,
    language TEXT,
    candidate_scores TEXT,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
);
INSERT INTO "routing_decisions" VALUES(1,2,2,'round_robin','fixture-agent','worker','implement','rust','[]','SYNTHETIC fixture route','2026-10-26T00:00:00Z');
CREATE TABLE runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'planned',
    planner_agent TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, team TEXT, integration_sha TEXT, base_sha TEXT, plan_revision INTEGER NOT NULL DEFAULT 1, recipe TEXT);
INSERT INTO "runs" VALUES(1,'SYNTHETIC V1 preservation workflow','completed','fixture-agent','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','{"planner":"fixture-agent","workers":["fixture-agent"],"reviewers":["fixture-agent"],"additional":{"architect":["fixture-agent"]}}','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,'{"id":"fixture-recipe","name":"SYNTHETIC retained recipe","roles":["planner","architect","worker","reviewer"],"routing":"round_robin","maxParallelTasks":2,"policyPresets":{"worker":"workspace_write"},"tools":[],"plannerGuidance":"SYNTHETIC fixture only","github":{"draft":true}}');
INSERT INTO "runs" VALUES(2,'SYNTHETIC V1 pending workflow','planned','fixture-agent','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','{"planner":"fixture-agent","workers":["fixture-agent"],"reviewers":["fixture-agent"],"additional":{"architect":["fixture-agent"]}}',NULL,NULL,1,NULL);
CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
INSERT INTO "schema_migrations" VALUES(1,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(2,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(3,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(4,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(5,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(6,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(7,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(8,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(9,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(10,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(11,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(12,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(13,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(14,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(15,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(16,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(17,'2026-10-26T00:00:00Z');
INSERT INTO "schema_migrations" VALUES(18,'2026-10-26T00:00:00Z');
CREATE TABLE task_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    agent TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    worktree_path TEXT NOT NULL,
    commit_sha TEXT,
    exit_code INTEGER,
    error TEXT,
    evidence TEXT,
    review TEXT, role TEXT, operation TEXT, source_base TEXT,
    UNIQUE(task_id, attempt_number)
);
INSERT INTO "task_attempts" VALUES(1,1,1,'fixture-agent','approved','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','.factory/worktrees/t1',NULL,0,NULL,NULL,NULL,'architect','advisory','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
INSERT INTO "task_attempts" VALUES(2,2,1,'fixture-agent','approved','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','.factory/worktrees/t2','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',0,NULL,'{"changedFiles":["src/main.rs"],"diffSummary":"SYNTHETIC patch","commitSha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","commands":["SYNTHETIC fixture command - never executed"],"acceptanceCriteria":["SYNTHETIC preservation criterion"],"workerExitCode":0,"artifacts":[],"diffPatch":"SYNTHETIC diff - not execution evidence"}','{"decision":"approve","reason":"SYNTHETIC fixture review - not a real review","feedback":[]}','worker','implement','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
CREATE TABLE task_dependencies (
    task_id INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    depends_on INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    PRIMARY KEY (task_id, depends_on)
);
INSERT INTO "task_dependencies" VALUES(2,1);
INSERT INTO "task_dependencies" VALUES(4,3);
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    objective TEXT NOT NULL,
    acceptance_criteria TEXT NOT NULL DEFAULT '[]',
    state TEXT NOT NULL DEFAULT 'pending',
    position INTEGER NOT NULL,
    worktree_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
, role TEXT, operation TEXT, agent_override TEXT);
INSERT INTO "tasks" VALUES(1,1,'Architecture','SYNTHETIC fixture task','["SYNTHETIC preservation criterion"]','completed',0,'.factory/worktrees/t1','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','architect','advisory',NULL);
INSERT INTO "tasks" VALUES(2,1,'Implementation','SYNTHETIC fixture task','["SYNTHETIC preservation criterion"]','completed',1,'.factory/worktrees/t2','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','worker','implement','fixture-agent');
INSERT INTO "tasks" VALUES(3,2,'Pending implementation','SYNTHETIC fixture task','["SYNTHETIC preservation criterion"]','ready',0,NULL,'2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','worker','implement',NULL);
INSERT INTO "tasks" VALUES(4,2,'Pending verification','SYNTHETIC fixture task','["SYNTHETIC preservation criterion"]','pending',1,NULL,'2026-10-26T00:00:00Z','2026-10-26T00:00:00Z','test_engineer','verify',NULL);
CREATE TABLE tool_grants (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES tasks(id) ON DELETE CASCADE,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE CASCADE,
    session_id INTEGER REFERENCES agent_sessions(id) ON DELETE CASCADE,
    agent TEXT NOT NULL,
    role TEXT NOT NULL,
    operation TEXT,
    allowed_tools TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);
INSERT INTO "tool_grants" VALUES('synthetic-revoked-grant','0000000000000000000000000000000000000000000000000000000000000000',1,2,2,3,'fixture-agent','worker','implement','["fixture.inspect"]','2026-10-26T00:00:00Z','2026-10-26T00:00:00Z');
CREATE TABLE tool_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    task_id INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
    attempt_id INTEGER REFERENCES task_attempts(id) ON DELETE SET NULL,
    session_id INTEGER REFERENCES agent_sessions(id) ON DELETE SET NULL,
    agent TEXT NOT NULL,
    role TEXT NOT NULL,
    server TEXT NOT NULL,
    tool TEXT NOT NULL,
    classification TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    duration_ms INTEGER,
    input_summary TEXT NOT NULL,
    output_summary TEXT,
    error TEXT
);
INSERT INTO "tool_invocations" VALUES(1,1,2,2,3,'fixture-agent','worker','fixture','inspect','read','success','2026-10-26T00:00:00Z',5,'SYNTHETIC input','SYNTHETIC output',NULL);
CREATE INDEX idx_tasks_run ON tasks(run_id);
CREATE INDEX idx_task_deps_dep ON task_dependencies(depends_on);
CREATE INDEX idx_sessions_run ON agent_sessions(run_id);
CREATE INDEX idx_tasks_run_state ON tasks(run_id, state);
CREATE INDEX idx_attempts_task ON task_attempts(task_id, attempt_number);
CREATE INDEX idx_sessions_attempt ON agent_sessions(attempt_id);
CREATE INDEX idx_sessions_status ON agent_sessions(status);
CREATE INDEX idx_sessions_mode_status ON agent_sessions(mode, status);
CREATE INDEX idx_artifacts_run_task ON role_artifacts(run_id, task_id);
CREATE INDEX idx_artifacts_task ON role_artifacts(task_id);
CREATE INDEX idx_plan_revisions_run ON plan_revisions(run_id, revision);
CREATE INDEX idx_github_links_issue ON github_links(repository, issue_number);
CREATE INDEX idx_integration_outcomes_task ON integration_outcomes(task_id);
CREATE INDEX idx_integration_outcomes_attempt ON integration_outcomes(attempt_id);
CREATE INDEX idx_routing_decisions_task ON routing_decisions(task_id);
CREATE INDEX idx_routing_decisions_attempt ON routing_decisions(attempt_id);
CREATE INDEX idx_mcp_tools_server ON mcp_tools(server_id);
CREATE INDEX idx_tool_grants_session ON tool_grants(session_id);
CREATE INDEX idx_tool_invocations_run ON tool_invocations(run_id);
CREATE INDEX idx_tool_invocations_task ON tool_invocations(task_id);
CREATE INDEX idx_tool_invocations_agent ON tool_invocations(agent);
DELETE FROM "sqlite_sequence";
INSERT INTO "sqlite_sequence" VALUES('runs',2);
INSERT INTO "sqlite_sequence" VALUES('tasks',4);
INSERT INTO "sqlite_sequence" VALUES('task_attempts',2);
INSERT INTO "sqlite_sequence" VALUES('agent_sessions',5);
INSERT INTO "sqlite_sequence" VALUES('role_artifacts',1);
INSERT INTO "sqlite_sequence" VALUES('plan_revisions',1);
INSERT INTO "sqlite_sequence" VALUES('github_links',1);
INSERT INTO "sqlite_sequence" VALUES('github_deliveries',1);
INSERT INTO "sqlite_sequence" VALUES('integration_outcomes',1);
INSERT INTO "sqlite_sequence" VALUES('routing_decisions',1);
INSERT INTO "sqlite_sequence" VALUES('tool_invocations',1);
COMMIT;
