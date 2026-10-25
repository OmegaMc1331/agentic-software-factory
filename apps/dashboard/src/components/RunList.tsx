import type { RunSummary } from "../types";
import { ProgressBar } from "./ProgressBar";

export function RunList({
  runs,
  onSelect,
  pageIndex,
  hasMore,
  loading,
  onNext,
  onPrevious,
}: {
  runs: RunSummary[];
  onSelect: (id: number) => void;
  pageIndex: number;
  hasMore: boolean;
  loading: boolean;
  onNext: () => void;
  onPrevious: () => void;
}) {
  const controls = (
    <div className="pagination" aria-label="Run history pagination">
      <button className="button" onClick={onPrevious} disabled={pageIndex === 0 || loading}>
        Previous
      </button>
      <span className="muted" aria-live="polite">
        {loading ? "Loading…" : `Page ${pageIndex + 1}`}
      </span>
      <button className="button" onClick={onNext} disabled={!hasMore || loading}>
        Next
      </button>
    </div>
  );

  if (runs.length === 0) {
    return (
      <div className="empty">
        <p className="empty-title">{pageIndex === 0 ? "No runs yet" : "No runs on this page"}</p>
        <p className="empty-body">
          {pageIndex === 0
            ? "Create a Workflow in Agent Graph to see it here."
            : "Return to the previous page to see older workflows."}
        </p>
        {pageIndex > 0 && controls}
      </div>
    );
  }

  return (
    <>
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
      {controls}
    </>
  );
}
