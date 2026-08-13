import { useEffect, useMemo, useState } from "react";
import { fetchRun } from "../api";
import type { GraphNode, RunDetail, Task } from "../types";
import { runMeta } from "../types";

type WorkflowTab = "overview" | "tasks" | "activity";

function progressLabel(detail: RunDetail | null, fallback: ReturnType<typeof runMeta>): string {
  const counts = detail
    ? {
        completed: detail.tasks.filter((task) => task.state === "completed").length,
        total: detail.tasks.length,
      }
    : fallback.counts;
  return `${counts.completed} / ${counts.total}`;
}

function taskFailure(task: Task, detail: RunDetail): string | null {
  const attempt = [...detail.attempts].reverse().find((candidate) => candidate.taskId === task.id);
  return attempt?.error ?? attempt?.review?.reason ?? null;
}

export function WorkflowInspector({
  node,
  onClose,
  onStart,
  onCancel,
  onRetry,
}: {
  node: GraphNode;
  onClose: () => void;
  onStart: (runId: number) => Promise<void>;
  onCancel: (runId: number) => Promise<void>;
  onRetry: (taskId: number) => Promise<void>;
}) {
  const meta = runMeta(node);
  const [tab, setTab] = useState<WorkflowTab>("overview");
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    const load = () => {
      fetchRun(meta.runId)
        .then((next) => {
          if (!active) return;
          setDetail(next);
          setError(null);
        })
        .catch((reason: Error) => {
          if (active) setError(reason.message);
        });
    };
    load();
    if (!["planning", "active"].includes(meta.status)) {
      return () => {
        active = false;
      };
    }
    const timer = window.setInterval(load, 2000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [meta.runId, meta.status]);

  const activity = useMemo(() => {
    if (!detail) return [];
    const entries = [
      ...detail.sessions.map((session) => ({
        at: session.startedAt,
        label: `${session.role[0].toUpperCase() + session.role.slice(1)} started — ${session.agent}`,
        status: session.status,
      })),
      ...detail.attempts.map((attempt) => ({
        at: attempt.finishedAt ?? attempt.startedAt,
        label: `Task #${attempt.taskId} attempt ${attempt.attemptNumber}`,
        status: attempt.status.replaceAll("_", " "),
      })),
    ];
    return entries.sort((a, b) => a.at.localeCompare(b.at));
  }, [detail]);

  const act = async (operation: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await operation();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const start = () => {
    const count = detail?.tasks.length ?? meta.counts.total;
    if (
      !window.confirm(
        `Start workflow?\n\n${count} planned task${count === 1 ? "" : "s"}\nWorker: ${meta.workerAgent ?? "Not configured"}\nReviewer: ${meta.reviewerAgent ?? "Not configured"}\n\nRepository changes will be made in isolated worktrees.`
      )
    ) {
      return;
    }
    void act(() => onStart(meta.runId));
  };

  return (
    <aside className="net-inspector workflow-inspector" aria-label={`${node.label} workflow`}>
      <div className="inspector-head">
        <div>
          <span className="inspector-kind">Workflow #{meta.runId}</span>
          <h3>{node.label}</h3>
        </div>
        <button className="inspector-close" onClick={onClose} aria-label="Close workflow inspector">
          x
        </button>
      </div>

      <div className="agent-console-tabs" role="tablist" aria-label="Workflow details">
        {(["overview", "tasks", "activity"] as WorkflowTab[]).map((name) => (
          <button
            key={name}
            role="tab"
            aria-selected={tab === name}
            className={tab === name ? "is-active" : ""}
            onClick={() => setTab(name)}
          >
            {name}
          </button>
        ))}
      </div>

      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}

      {tab === "overview" && (
        <div className="workflow-panel">
          <dl className="inspector-list">
            <div>
              <dt>Status</dt>
              <dd>{detail?.run.status ?? meta.status}</dd>
            </div>
            <div>
              <dt>Planner</dt>
              <dd>{meta.plannerAgent ?? "Not configured"}</dd>
            </div>
            <div>
              <dt>Progress</dt>
              <dd>{progressLabel(detail, meta)} tasks</dd>
            </div>
            <div>
              <dt>Objective</dt>
              <dd>{meta.objective}</dd>
            </div>
          </dl>
          <div className="workflow-actions">
            {meta.status === "planned" && (
              <button className="button button-primary" onClick={start} disabled={busy}>
                Start
              </button>
            )}
            {["planning", "active"].includes(meta.status) && (
              <button
                className="button inspector-delete"
                onClick={() =>
                  window.confirm("Cancel this workflow and stop its current agent session?") &&
                  void act(() => onCancel(meta.runId))
                }
                disabled={busy}
              >
                Cancel
              </button>
            )}
          </div>
        </div>
      )}

      {tab === "tasks" && (
        <div className="workflow-task-list">
          {!detail && <p className="inspector-hint">Loading the persisted plan…</p>}
          {detail?.tasks.map((task) => {
            const failure = taskFailure(task, detail);
            return (
              <article key={task.id} className="workflow-task-row">
                <div>
                  <strong>
                    #{task.id} {task.title}
                  </strong>
                  <span>{task.state}</span>
                </div>
                <p>{task.objective}</p>
                <ul>
                  {task.acceptanceCriteria.map((criterion) => (
                    <li key={criterion}>{criterion}</li>
                  ))}
                </ul>
                <small>
                  Dependencies:{" "}
                  {task.dependencies.length
                    ? task.dependencies.map((id) => `#${id}`).join(", ")
                    : "none"}
                </small>
                {failure && <p className="inline-error">{failure}</p>}
                {["failed", "blocked"].includes(task.state) && (
                  <button
                    className="button"
                    onClick={() => void act(() => onRetry(task.id))}
                    disabled={busy}
                  >
                    Retry task
                  </button>
                )}
              </article>
            );
          })}
        </div>
      )}

      {tab === "activity" && (
        <ol className="workflow-activity">
          {activity.length === 0 && <li>No activity recorded yet.</li>}
          {activity.map((entry, index) => (
            <li key={`${entry.at}:${index}`}>
              <time>
                {new Date(entry.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
              </time>
              <span>{entry.label}</span>
              <small>{entry.status}</small>
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
