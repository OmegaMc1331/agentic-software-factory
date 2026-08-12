export interface RunOption {
  id: number;
  label: string;
}

export function GraphToolbar({
  runOptions,
  runFilter,
  onRunFilter,
  showTasks,
  onShowTasks,
  showDependencies,
  onShowDependencies,
  live,
  onLive,
  onFit,
  onCenter,
  counts,
}: {
  runOptions: RunOption[];
  runFilter: number | null;
  onRunFilter: (id: number | null) => void;
  showTasks: boolean;
  onShowTasks: (value: boolean) => void;
  showDependencies: boolean;
  onShowDependencies: (value: boolean) => void;
  live: boolean;
  onLive: (value: boolean) => void;
  onFit: () => void;
  onCenter: () => void;
  counts: { runs: number; agents: number; tasks: number };
}) {
  return (
    <div className="net-toolbar">
      <h2 className="net-toolbar-title">Factory Network</h2>

      {runOptions.length > 1 && (
        <select
          className="net-select"
          aria-label="Run"
          value={runFilter === null ? "" : String(runFilter)}
          onChange={(event) =>
            onRunFilter(event.target.value === "" ? null : Number(event.target.value))
          }
        >
          <option value="">All runs</option>
          {runOptions.map((run) => (
            <option key={run.id} value={String(run.id)}>
              {run.label}
            </option>
          ))}
        </select>
      )}

      <label className="net-toggle">
        <input
          type="checkbox"
          checked={showTasks}
          onChange={(event) => onShowTasks(event.target.checked)}
        />
        Tasks
      </label>
      <label className="net-toggle">
        <input
          type="checkbox"
          checked={showDependencies}
          onChange={(event) => onShowDependencies(event.target.checked)}
        />
        Dependencies
      </label>

      <span className="net-toolbar-spacer" />

      <span className="net-counts">
        {counts.agents} agent{counts.agents === 1 ? "" : "s"} · {counts.runs} run
        {counts.runs === 1 ? "" : "s"} · {counts.tasks} task{counts.tasks === 1 ? "" : "s"}
      </span>

      <button className="net-btn" onClick={onFit}>
        Fit
      </button>
      <button className="net-btn" onClick={onCenter}>
        Center
      </button>
      <button
        className={live ? "net-btn net-btn-live" : "net-btn"}
        aria-pressed={live}
        onClick={() => onLive(!live)}
      >
        <span className={live ? "net-live-dot net-live-dot--on" : "net-live-dot"} />
        {live ? "Live" : "Paused"}
      </button>
    </div>
  );
}
