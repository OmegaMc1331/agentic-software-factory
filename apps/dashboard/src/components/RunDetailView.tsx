import type { RunDetail } from "../types";
import { STAGE_META, type WorkflowStage } from "../types";
import { StatusBadge } from "./StatusBadge";
import { TaskGraph } from "./TaskGraph";
import { ArtifactList } from "./ArtifactList";
import { truncate } from "../layout";

export function RunDetailView({ detail, onBack }: { detail: RunDetail; onBack: () => void }) {
  const tasks = detail.tasks;
  const completed = tasks.filter((t) => t.state === "completed").length;
  const pct = tasks.length === 0 ? 0 : Math.round((completed / tasks.length) * 100);

  const artifactsForTask = (taskId: number, dependencies: number[]) => {
    const produced = detail.artifacts.filter((artifact) => artifact.taskId === taskId);
    const consumed = detail.artifacts.filter(
      (artifact) => artifact.taskId !== null && dependencies.includes(artifact.taskId)
    );
    return { produced, consumed };
  };

  return (
    <div className="detail">
      <button className="button" onClick={onBack}>
        ← All runs
      </button>

      <div className="detail-header">
        <h2 className="detail-title">Run #{detail.run.id}</h2>
        <span className="status-text">{detail.run.status}</span>
        <span className="muted">· {pct}% complete</span>
      </div>

      <p className="detail-objective">{detail.run.objective}</p>

      <div className="meta-grid">
        <div className="meta-cell">
          <span className="meta-label">Planner</span>
          <code className="meta-value">{detail.run.plannerAgent ?? "–"}</code>
        </div>
        <div className="meta-cell">
          <span className="meta-label">Created</span>
          <span className="meta-value muted">
            {new Date(detail.run.createdAt).toLocaleString()}
          </span>
        </div>
      </div>

      {detail.stages.length > 0 && (
        <section className="section">
          <h3 className="section-title">Stages</h3>
          <ul className="stage-list">
            {detail.stages.map((stage) => {
              const accent = STAGE_META[stageKeyToWorkflowStage(stage.key)];
              return (
                <li key={stage.key} className={`stage--${stage.state}`}>
                  <span style={{ color: accent.color }}>{stage.label}</span>
                  <strong>{stage.state}</strong>
                  <small>
                    {stage.completed}/{stage.total}
                  </small>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <section className="section">
        <h3 className="section-title">Task graph</h3>
        <div className="graph-wrap">
          <TaskGraph tasks={tasks} />
        </div>
      </section>

      <section className="section">
        <h3 className="section-title">
          Tasks <span className="muted">({tasks.length})</span>
        </h3>
        <table className="table">
          <thead>
            <tr>
              <th>#</th>
              <th>Title</th>
              <th>Role</th>
              <th>Operation</th>
              <th>Objective</th>
              <th>Status</th>
              <th>Depends on</th>
            </tr>
          </thead>
          <tbody>
            {tasks.map((task) => {
              const artifacts = artifactsForTask(task.id, task.dependencies);
              return (
                <tr key={task.id}>
                  <td>#{task.id}</td>
                  <td>
                    {task.title}
                    {artifacts.produced.length > 0 && (
                      <span className="muted">
                        {" "}
                        · {artifacts.produced.length} artifact
                        {artifacts.produced.length === 1 ? "" : "s"}
                      </span>
                    )}
                  </td>
                  <td>{task.role ?? "worker"}</td>
                  <td>
                    <span className="op-chip">{task.operation ?? "implement"}</span>
                  </td>
                  <td className="run-objective">{truncate(task.objective, 48)}</td>
                  <td>
                    <StatusBadge state={task.state} />
                  </td>
                  <td className="muted">
                    {task.dependencies.length === 0
                      ? "–"
                      : task.dependencies.map((d) => `#${d}`).join(", ")}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>

      {detail.artifacts.length > 0 && (
        <section className="section">
          <h3 className="section-title">
            Role artifacts <span className="muted">({detail.artifacts.length})</span>
          </h3>
          <ArtifactList artifacts={detail.artifacts} />
        </section>
      )}
    </div>
  );
}

function stageKeyToWorkflowStage(key: string): WorkflowStage {
  switch (key) {
    case "analysis":
      return "analysis";
    case "implementation":
      return "implementation";
    case "verification":
      return "verification";
    case "review":
      return "review";
    case "post_process":
      return "post_processing";
    default:
      return "planning";
  }
}
