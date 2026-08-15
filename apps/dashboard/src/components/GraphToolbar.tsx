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
  showRoles,
  onShowRoles,
  live,
  onLive,
  onAdd,
  onFit,
  onCenter,
  onZoomOut,
  onZoomIn,
  onResetLayout,
  zoom,
}: {
  runOptions: RunOption[];
  runFilter: number | null;
  onRunFilter: (id: number | null) => void;
  showTasks: boolean;
  onShowTasks: (value: boolean) => void;
  showDependencies: boolean;
  onShowDependencies: (value: boolean) => void;
  showRoles: boolean;
  onShowRoles: (value: boolean) => void;
  live: boolean;
  onLive: (value: boolean) => void;
  onAdd: () => void;
  onFit: () => void;
  onCenter: () => void;
  onZoomOut: () => void;
  onZoomIn: () => void;
  onResetLayout: () => void;
  zoom: number;
}) {
  return (
    <div className="net-toolbar">
      <div className="net-toolbar-heading">
        <h2 className="net-toolbar-title">Agent Graph</h2>
        <span className="net-toolbar-subtitle">Factory topology and operations</span>
      </div>

      <div className="net-toolgroup" role="toolbar" aria-label="Graph viewport">
        <button className="net-btn net-btn--primary" onClick={onAdd} aria-label="Add graph node">
          <span aria-hidden="true">+</span>
          Add
        </button>
        <button className="net-btn" onClick={onFit} title="Fit graph (F)">
          Fit
        </button>
        <button className="net-btn" onClick={onCenter} title="Center graph">
          Center
        </button>
        <button className="net-btn net-btn--icon" onClick={onZoomOut} aria-label="Zoom out">
          -
        </button>
        <output className="net-zoom" aria-label="Current zoom">
          {Math.round(zoom * 100)}%
        </output>
        <button className="net-btn net-btn--icon" onClick={onZoomIn} aria-label="Zoom in">
          +
        </button>
        <button className="net-btn net-btn--reset" onClick={onResetLayout}>
          Reset layout
        </button>
      </div>

      <div className="net-toolbar-options">
        {runOptions.length > 1 && (
          <select
            className="net-select"
            aria-label="Visible run"
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
        <label className="net-toggle">
          <input
            type="checkbox"
            checked={showRoles}
            onChange={(event) => onShowRoles(event.target.checked)}
          />
          Roles
        </label>
        <button
          className={live ? "net-btn net-btn-live" : "net-btn"}
          aria-pressed={live}
          onClick={() => onLive(!live)}
        >
          <span className={live ? "net-live-dot net-live-dot--on" : "net-live-dot"} />
          {live ? "Live" : "Paused"}
        </button>
      </div>
    </div>
  );
}
