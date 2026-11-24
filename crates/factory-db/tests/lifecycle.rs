use factory_db::FactoryDb;
use factory_types::{MissionLifecycleState as State, RunStatus};

const PROJECT: &str = "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SESSION: &str = "s_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
#[test]
fn noop_suspends_are_coalesced_and_resume_history_has_a_reserved_safety_slot() {
    let (dir, db, run) = fixture();
    db.request_mission_suspend(run, 1, "first", "user_requested_suspend")
        .unwrap();
    for n in 0..100 {
        db.request_mission_suspend(run, 2, &format!("noop-{n}"), "user_requested_suspend")
            .unwrap();
    }
    let mut connection = rusqlite::Connection::open(dir.path().join("db.sqlite3")).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM lifecycle_commands WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    let tx = connection.transaction().unwrap();
    for n in 0..1022 {
        tx.execute(
            "INSERT INTO lifecycle_commands VALUES(?1,?2,'resume',2)",
            rusqlite::params![run, format!("seed-{n}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
        .unwrap();
    db.save_mission_checkpoint(run, 2, vec![], None).unwrap();
    db.lifecycle_transition(run, 2, State::Quiescing, State::Suspended, None)
        .unwrap();
    assert!(db
        .begin_mission_resume(run, 2, "over-budget-resume")
        .is_err());
    assert!(db.lifecycle_fenced(run).unwrap());
    let (other, db, run) = fixture();
    let mut connection = rusqlite::Connection::open(other.path().join("db.sqlite3")).unwrap();
    let tx = connection.transaction().unwrap();
    for n in 0..1023 {
        tx.execute(
            "INSERT INTO lifecycle_commands VALUES(?1,?2,'resume',1)",
            rusqlite::params![run, format!("seed-{n}")],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    assert_eq!(
        db.request_mission_suspend(run, 1, "mandatory-safety-fence", "user_requested_suspend")
            .unwrap()
            .state,
        State::SuspendRequested
    );
    assert!(db.lifecycle_fenced(run).unwrap());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM lifecycle_commands WHERE run_id=?1",
                [run],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1024
    );
}

#[test]
fn session_closure_and_newer_suspension_win_over_pending_resume_commit() {
    for close_session in [true, false] {
        let (_dir, db, run) = fixture();
        db.request_mission_suspend(run, 1, "initial", "user_requested_suspend")
            .unwrap();
        db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
            .unwrap();
        db.save_mission_checkpoint(run, 2, vec![], None).unwrap();
        db.lifecycle_transition(run, 2, State::Quiescing, State::Suspended, None)
            .unwrap();
        assert!(db.begin_mission_resume(run, 2, "pending-resume").unwrap());
        if close_session {
            db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
                .unwrap();
        } else {
            let next = db
                .request_mission_suspend(run, 3, "newer-suspend", "user_requested_suspend")
                .unwrap();
            assert_eq!(next.generation, 4);
            assert_eq!(next.state, State::SuspendRequested);
        }
        assert!(db.commit_mission_resume(run, 3).is_err());
        assert!(db.lifecycle_fenced(run).unwrap());
        assert_eq!(db.active_dispatch_count(run).unwrap(), 0);
    }
}

fn fixture() -> (tempfile::TempDir, FactoryDb, i64) {
    let dir = tempfile::tempdir().unwrap();
    let db = FactoryDb::open(&dir.path().join("db.sqlite3")).unwrap();
    db.register_workspace_session(PROJECT, SESSION).unwrap();
    let run = db.create_run("lifecycle", None).unwrap();
    db.bind_workspace_run(run.id, PROJECT, SESSION).unwrap();
    (dir, db, run.id)
}
#[test]
fn ownership_is_durable_and_never_reassigned_by_navigation() {
    let (dir, db, run) = fixture();
    assert!(db.bind_workspace_run(run, PROJECT, SESSION).is_err());
    drop(db);
    let db = FactoryDb::open(&dir.path().join("db.sqlite3")).unwrap();
    let life = db.mission_lifecycle(run).unwrap().unwrap();
    assert_eq!(life.workspace_session_id, SESSION);
    assert_eq!(life.project_id, PROJECT);
}
#[test]
fn explicit_close_uses_revision_and_reopen_fences_stale_close() {
    let (_dir, db, _) = fixture();
    let closed = db
        .set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    assert!(closed.closed);
    assert_eq!(closed.generation, 2);
    assert!(
        db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
            .unwrap()
            .closed
    );
    assert!(db
        .set_workspace_session_closed(PROJECT, SESSION, 1, false)
        .is_err());
    db.set_workspace_session_closed(PROJECT, SESSION, 2, false)
        .unwrap();
    assert!(db
        .set_workspace_session_closed(PROJECT, SESSION, 2, true)
        .is_err());
}
#[test]
fn suspended_dispatch_cannot_acquire_scheduler_or_task_ownership() {
    let (_dir, db, run) = fixture();
    db.set_run_status(run, RunStatus::Active).unwrap();
    let life = db
        .request_mission_suspend(run, 1, "first", "user_requested_suspend")
        .unwrap();
    assert_eq!(life.generation, 2);
    assert!(db.lifecycle_fenced(run).unwrap());
    assert!(!db.acquire_scheduler(run, "stale-scheduler").unwrap());
    assert_eq!(
        db.request_mission_suspend(run, 1, "first", "user_requested_suspend")
            .unwrap()
            .generation,
        2
    );
    assert!(db
        .request_mission_suspend(run, 1, "second", "user_requested_suspend")
        .is_err());
}
#[test]
fn checkpoint_survives_restart_and_is_project_scoped() {
    let (dir, db, run) = fixture();
    db.request_mission_suspend(run, 1, "s", "user_requested_suspend")
        .unwrap();
    db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
        .unwrap();
    let checkpoint = db.save_mission_checkpoint(run, 2, vec![], None).unwrap();
    db.lifecycle_transition(run, 2, State::Quiescing, State::Suspended, None)
        .unwrap();
    drop(db);
    let db = FactoryDb::open(&dir.path().join("db.sqlite3")).unwrap();
    db.mark_lifecycle_runtime_interruption().unwrap();
    assert_eq!(
        db.mission_lifecycle(run).unwrap().unwrap().state,
        State::Suspended
    );
    assert_eq!(db.mission_checkpoints(run, None).unwrap()[0], checkpoint);
    assert!(db
        .workspace_session_lifecycle("p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", SESSION)
        .is_err());
}
#[test]
fn checkpoint_requires_drained_scheduler_and_valid_transition() {
    let (_dir, db, run) = fixture();
    db.set_run_status(run, RunStatus::Active).unwrap();
    assert!(db.acquire_scheduler(run, "owner").unwrap());
    db.request_mission_suspend(run, 1, "s", "user_requested_suspend")
        .unwrap();
    assert!(db.save_mission_checkpoint(run, 2, vec![], None).is_err());
    db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
        .unwrap();
    assert!(db.save_mission_checkpoint(run, 2, vec![], None).is_err());
    db.finish_scheduler("owner", "suspended").unwrap();
    assert!(db.save_mission_checkpoint(run, 2, vec![], None).is_ok());
    assert!(db
        .lifecycle_transition(run, 1, State::Quiescing, State::Suspended, None)
        .is_err());
    assert!(db
        .lifecycle_transition(run, 2, State::Quiescing, State::Active, None)
        .is_err());
}
#[test]
fn interrupted_runtime_is_not_claimed_to_be_safely_suspended() {
    let (_dir, db, run) = fixture();
    db.mark_lifecycle_runtime_interruption().unwrap();
    let life = db.mission_lifecycle(run).unwrap().unwrap();
    assert_eq!(life.state, State::RecoveryRequired);
    assert_eq!(life.reason.as_deref(), Some("runtime_interruption"));
    assert!(life.checkpoint_id.is_none());
}
#[test]
fn completed_mission_wins_suspension_race() {
    let (_dir, db, run) = fixture();
    db.set_run_status(run, RunStatus::Completed).unwrap();
    assert_eq!(
        db.request_mission_suspend(run, 1, "s", "user_requested_suspend")
            .unwrap()
            .state,
        State::Active
    );
    db.settle_mission_lifecycle(run).unwrap();
    assert_eq!(
        db.mission_lifecycle(run).unwrap().unwrap().state,
        State::Settled
    );
    assert!(db.begin_mission_resume(run, 1, "r").is_err());
    let (_other, db, run) = fixture();
    db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    db.request_mission_suspend(run, 1, "race-close", "explicit_session_close")
        .unwrap();
    db.set_run_status(run, RunStatus::Completed).unwrap();
    db.complete_lifecycle_race(run, 2).unwrap();
    db.complete_lifecycle_race(run, 2).unwrap();
    assert_eq!(
        db.mission_lifecycle(run).unwrap().unwrap().state,
        State::Settled
    );
    let session = db.workspace_session_lifecycle(PROJECT, SESSION).unwrap();
    assert_eq!(
        session
            .events
            .iter()
            .filter(|e| e["eventType"] == "session.closed")
            .count(),
        1
    );
}
#[test]
fn duplicate_resume_and_old_suspend_cannot_start_two_generations() {
    let (_dir, db, run) = fixture();
    db.request_mission_suspend(run, 1, "s", "user_requested_suspend")
        .unwrap();
    db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
        .unwrap();
    db.save_mission_checkpoint(run, 2, vec![], None).unwrap();
    db.lifecycle_transition(run, 2, State::Quiescing, State::Suspended, None)
        .unwrap();
    assert!(db.begin_mission_resume(run, 2, "r").unwrap());
    assert!(!db.begin_mission_resume(run, 2, "r").unwrap());
    assert!(db.begin_mission_resume(run, 2, "r2").is_err());
    assert!(db
        .request_mission_suspend(run, 2, "s2", "user_requested_suspend")
        .is_err());
}
#[test]
fn closed_session_cannot_accept_new_mission_or_resume() {
    let (_dir, db, run) = fixture();
    db.set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    let second = db.create_run("new", None).unwrap();
    assert!(db.bind_workspace_run(second.id, PROJECT, SESSION).is_err());
    db.request_mission_suspend(run, 1, "s", "explicit_session_close")
        .unwrap();
    db.lifecycle_transition(run, 2, State::SuspendRequested, State::Quiescing, None)
        .unwrap();
    db.save_mission_checkpoint(run, 2, vec![], None).unwrap();
    db.lifecycle_transition(run, 2, State::Quiescing, State::Suspended, None)
        .unwrap();
    assert!(db.begin_mission_resume(run, 2, "r").is_err());
}
#[test]
fn legacy_workflows_do_not_require_browser_presence() {
    let dir = tempfile::tempdir().unwrap();
    let db = FactoryDb::open(&dir.path().join("db.sqlite3")).unwrap();
    let run = db.create_run("CLI", None).unwrap();
    assert!(db.mission_lifecycle(run.id).unwrap().is_none());
    assert!(!db.lifecycle_fenced(run.id).unwrap());
    db.set_run_status(run.id, RunStatus::Active).unwrap();
    assert!(db.acquire_scheduler(run.id, "cli").unwrap());
}

#[test]
fn full_conversation_notification_storage_does_not_rollback_the_safety_fence() {
    let (dir, db, _) = fixture();
    db.conversation_create("c_test", PROJECT, "Lifecycle budget")
        .unwrap();
    let (command, _) = db
        .conversation_accept(
            "c_test",
            "objective",
            "fingerprint",
            "chat",
            "Objective",
            &serde_json::json!({"workspace_session_id":SESSION}),
        )
        .unwrap();
    db.conversation_state(
        command.id,
        "processing",
        "Processing",
        &serde_json::json!({}),
        None,
    )
    .unwrap();
    let run = db
        .conversation_begin_run(
            command.id,
            "Objective",
            &factory_types::WorkflowTeam {
                planner: "planner".into(),
                workers: vec!["worker".into()],
                reviewers: vec!["reviewer".into()],
                additional: Default::default(),
            },
            None,
        )
        .unwrap();
    let fault = rusqlite::Connection::open(dir.path().join("db.sqlite3")).unwrap();
    fault.execute_batch("CREATE TRIGGER refuse_notifications BEFORE INSERT ON conversation_events BEGIN SELECT RAISE(FAIL,'controlled notification storage refusal'); END;").unwrap();
    db.request_mission_suspend(
        run.id,
        1,
        "stop-without-presentation",
        "user_requested_suspend",
    )
    .unwrap();
    assert!(db.lifecycle_fenced(run.id).unwrap());
    assert_eq!(
        db.mission_lifecycle(run.id).unwrap().unwrap().state,
        State::SuspendRequested
    );
    assert!(db
        .recent_scheduler_events(run.id)
        .unwrap()
        .iter()
        .any(|e| e.event_type == "lifecycle_presentation_blocked"));
}

#[test]
fn session_closure_and_reopening_have_distinct_idempotent_audit_events() {
    let (_dir, db, _) = fixture();
    let closed = db
        .set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    assert!(closed
        .events
        .iter()
        .any(|e| e["eventType"] == "session.close_requested"));
    let repeated = db
        .set_workspace_session_closed(PROJECT, SESSION, 1, true)
        .unwrap();
    assert_eq!(closed.events, repeated.events);
    let open = db
        .set_workspace_session_closed(PROJECT, SESSION, 2, false)
        .unwrap();
    assert!(open
        .events
        .iter()
        .any(|e| e["eventType"] == "session.reopened"));
}
