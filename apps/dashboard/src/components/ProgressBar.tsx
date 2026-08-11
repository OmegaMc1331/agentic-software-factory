import { progress } from "../api";

export function ProgressBar({ completed, total }: { completed: number; total: number }) {
  const pct = progress({ completed, total });
  return (
    <div className="progress">
      <div className="progress-content">
        <div
          className="progress-bar"
          data-state={pct === 1 ? "complete" : total === 0 ? "empty" : "active"}
          style={{ width: `${Math.round(pct * 100)}%` }}
        />
      </div>
      <span className="progress-text">
        {completed}/{total}
      </span>
    </div>
  );
}
