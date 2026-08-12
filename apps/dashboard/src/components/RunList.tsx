import type { RunSummary } from "../types";
import { ProgressBar } from "./ProgressBar";

export function RunList({
  runs,
  onSelect,
}: {
  runs: RunSummary[];
  onSelect: (id: number) => void;
}) {
  if (runs.length === 0) {
    return (
      <div className="empty">
        <p className="empty-title">No runs yet</p>
        <p className="empty-body">
          Start a run with <code>factory run</code> to see it here.
        </p>
      </div>
    );
  }

  return (
    <table className="table">
      <thead>
        <tr>
          <th>Run</th>
          <th>Objective</th>
          <th>Status</th>
          <th>Planner</th>
          <th>Progress</th>
          <th>Created</th>
        </tr>
      </thead>
      <tbody>
        {runs.map((run) => (
          <tr key={run.id} onClick={() => onSelect(run.id)} className="clickable">
            <td>
              <span className="run-link">#{run.id}</span>
            </td>
            <td className="run-objective">{run.objective}</td>
            <td>
              <span className="status-text">{run.status}</span>
            </td>
            <td>
              <code>{run.plannerAgent ?? "–"}</code>
            </td>
            <td>
              <ProgressBar completed={run.counts.completed} total={run.counts.total} />
            </td>
            <td className="muted">{new Date(run.createdAt).toLocaleString()}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
