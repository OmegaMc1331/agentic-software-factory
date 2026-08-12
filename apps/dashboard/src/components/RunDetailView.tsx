import type { RunDetail } from "../types";
import { StatusBadge } from "./StatusBadge";
import { TaskGraph } from "./TaskGraph";
import { truncate } from "../layout";

export function RunDetailView({ detail, onBack }: { detail: RunDetail; onBack: () => void }) {
  const tasks = detail.tasks;
  const completed = tasks.filter((t) => t.state === "completed").length;
  const pct = tasks.length === 0 ? 0 : Math.round((completed / tasks.length) * 100);

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
              <th>Objective</th>
              <th>Status</th>
              <th>Depends on</th>
              <th>Worktree</th>
            </tr>
          </thead>
          <tbody>
            {tasks.map((task) => (
              <tr key={task.id}>
                <td>#{task.id}</td>
                <td>{task.title}</td>
                <td className="run-objective">{truncate(task.objective, 48)}</td>
                <td>
                  <StatusBadge state={task.state} />
                </td>
                <td className="muted">
                  {task.dependencies.length === 0
                    ? "–"
                    : task.dependencies.map((d) => `#${d}`).join(", ")}
                </td>
                <td className="muted">{task.worktreePath ?? "–"}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </div>
  );
}
