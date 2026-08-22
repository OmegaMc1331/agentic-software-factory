import { useCallback, useEffect, useState } from "react";
import { fetchAgentPerformance, fetchPerformanceOverview } from "../api";
import {
  formatAttempts,
  formatDurationMs,
  formatPercent,
  formatRate,
  formatRateWithInterval,
  formatSignedPp,
  languageLabel,
} from "../performanceFormat";
import type {
  AgentPerformanceDetail,
  AgentPerformanceSummary,
  PerformanceBreakdownEntry,
  PerformanceFilters,
  PerformanceOverview,
} from "../types";

const WINDOWS: { value: string; label: string }[] = [
  { value: "all", label: "All time" },
  { value: "30d", label: "Last 30 days" },
  { value: "7d", label: "Last 7 days" },
];

function Select({
  id,
  label,
  value,
  options,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (next: string) => void;
}) {
  return (
    <label className="performance-filter" htmlFor={id}>
      <span>{label}</span>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)}>
        <option value="">All</option>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function OverviewTable({
  agents,
  onSelect,
  selected,
}: {
  agents: AgentPerformanceSummary[];
  onSelect: (agent: string) => void;
  selected: string | null;
}) {
  if (agents.length === 0) {
    return (
      <div className="empty">
        <p className="empty-title">No performance data</p>
        <p className="empty-body">Run a workflow; measured history appears here.</p>
      </div>
    );
  }
  return (
    <table className="table" aria-label="Agent performance overview">
      <thead>
        <tr>
          <th>Agent</th>
          <th>Tasks</th>
          <th>1st-pass</th>
          <th>Avg attempts</th>
          <th>Median execution</th>
          <th>Terminal failures</th>
        </tr>
      </thead>
      <tbody>
        {agents.map(({ agent, metrics }) => (
          <tr
            key={agent}
            onClick={() => onSelect(agent)}
            className={agent === selected ? "clickable performance-selected" : "clickable"}
          >
            <td>
              <span className="run-link">{agent}</span>
            </td>
            <td>{metrics.tasksAttempted}</td>
            <td>{formatRate(metrics.firstPassApproval)}</td>
            <td>{formatAttempts(metrics.attemptsPerTask)}</td>
            <td>{formatDurationMs(metrics.executionDuration.medianMs)}</td>
            <td>{formatRate(metrics.terminalFailure)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function BreakdownTable({
  title,
  entries,
}: {
  title: string;
  entries: PerformanceBreakdownEntry[];
}) {
  if (entries.length === 0) return null;
  return (
    <div className="performance-breakdown">
      <h4>By {title}</h4>
      <table className="table">
        <thead>
          <tr>
            <th>{title}</th>
            <th>Tasks</th>
            <th>1st-pass</th>
            <th>Avg attempts</th>
            <th>Median</th>
          </tr>
        </thead>
        <tbody>
          {entries.map(({ key, metrics }) => (
            <tr key={key}>
              <td>{title === "language" ? languageLabel(key) : key}</td>
              <td>{metrics.tasksAttempted}</td>
              <td>{formatRate(metrics.firstPassApproval)}</td>
              <td>{formatAttempts(metrics.attemptsPerTask)}</td>
              <td>{formatDurationMs(metrics.executionDuration.medianMs)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function OutcomeList({ detail }: { detail: AgentPerformanceDetail }) {
  const counts = detail.summary.metrics.outcomeCounts;
  const rows: [string, number][] = [
    ["Approved (eventual)", counts.approved],
    ["Approved on 1st pass", counts.firstPassApproved],
    ["Changes requested", counts.changesRequested],
    ["Agent failures", counts.agentFailed],
    ["Integration conflicts", counts.integrationConflict],
    ["Cancelled", counts.cancelled],
    ["Interrupted", counts.interrupted],
    ["Policy blocked", counts.policyBlocked],
    ["Configuration errors", counts.configurationError],
    ["In progress", counts.inProgress],
  ];
  return (
    <div className="performance-outcomes">
      <h4>Outcomes</h4>
      <dl>
        {rows
          .filter(([, count]) => count > 0)
          .map(([label, count]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd>{count}</dd>
            </div>
          ))}
      </dl>
      <p className="muted performance-note">
        Cancellations, interruptions, policy and configuration outcomes are excluded from the
        quality rates above.
      </p>
    </div>
  );
}

function ReasonList({
  title,
  reasons,
}: {
  title: string;
  reasons: { reason: string; count: number }[];
}) {
  if (reasons.length === 0) return null;
  return (
    <div className="performance-reasons">
      <h4>{title}</h4>
      <ul>
        {reasons.map((entry) => (
          <li key={entry.reason}>
            <span>{entry.reason}</span>
            <span className="muted">×{entry.count}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function AgentDetail({ detail, onClose }: { detail: AgentPerformanceDetail; onClose: () => void }) {
  const metrics = detail.summary.metrics;
  const trend = detail.trend;
  const weekly = trend.weekly;
  return (
    <section
      className="performance-detail"
      aria-label={`Performance detail for ${detail.summary.agent}`}
    >
      <div className="performance-detail-head">
        <h3>{detail.summary.agent}</h3>
        <button className="button" onClick={onClose}>
          Back to overview
        </button>
      </div>

      {detail.routing && (
        <p
          className={
            detail.routing.usedForRouting ? "performance-routing is-used" : "performance-routing"
          }
        >
          <strong>Used for routing:</strong> {detail.routing.usedForRouting ? "Yes" : "No"} —{" "}
          {detail.routing.note}
        </p>
      )}

      <dl className="performance-summary-grid">
        <div>
          <dt>Tasks</dt>
          <dd>{metrics.tasksAttempted}</dd>
        </div>
        <div>
          <dt>First-pass approval</dt>
          <dd>{formatRateWithInterval(metrics.firstPassApproval)}</dd>
        </div>
        <div>
          <dt>Eventual approval</dt>
          <dd>{formatRateWithInterval(metrics.eventualApproval)}</dd>
        </div>
        <div>
          <dt>Avg attempts / successful task</dt>
          <dd>{formatAttempts(metrics.avgAttemptsPerSuccessful)}</dd>
        </div>
      </dl>

      <div className="performance-columns">
        <div>
          <h4>Durations</h4>
          <table className="table">
            <thead>
              <tr>
                <th>Phase</th>
                <th>Median</th>
                <th>p95</th>
                <th>n</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>Agent execution</td>
                <td>{formatDurationMs(metrics.executionDuration.medianMs)}</td>
                <td>{formatDurationMs(metrics.executionDuration.p95Ms)}</td>
                <td>{metrics.executionDuration.samples}</td>
              </tr>
              <tr>
                <td>Review</td>
                <td>{formatDurationMs(metrics.reviewDuration.medianMs)}</td>
                <td>{formatDurationMs(metrics.reviewDuration.p95Ms)}</td>
                <td>{metrics.reviewDuration.samples}</td>
              </tr>
              <tr>
                <td>Total task</td>
                <td>{formatDurationMs(metrics.totalDuration.medianMs)}</td>
                <td>{formatDurationMs(metrics.totalDuration.p95Ms)}</td>
                <td>{metrics.totalDuration.samples}</td>
              </tr>
            </tbody>
          </table>
          {metrics.executionDuration.approximateSamples > 0 && (
            <p className="muted performance-note">
              {metrics.executionDuration.approximateSamples} sample(s) derive from attempt wall time
              (no session timer recorded).
            </p>
          )}

          <h4>Integration</h4>
          <table className="table">
            <thead>
              <tr>
                <th>Clean</th>
                <th>Rebased</th>
                <th>Conflicts</th>
                <th>Conflict rate</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <td>{metrics.integration.clean}</td>
                <td>{metrics.integration.rebased}</td>
                <td>{metrics.integration.conflict}</td>
                <td>{formatRate(metrics.integration.conflictRate)}</td>
              </tr>
            </tbody>
          </table>
          <p className="muted performance-note">
            Integration conflicts are reported separately and do not count as agent failures.
          </p>
        </div>
        <div>
          <OutcomeList detail={detail} />
        </div>
      </div>

      <BreakdownTable title="role" entries={detail.byRole} />
      <BreakdownTable title="operation" entries={detail.byOperation} />
      <BreakdownTable title="language" entries={detail.byLanguage} />

      <div className="performance-trend">
        <h4>Recent trend</h4>
        <table className="table">
          <thead>
            <tr>
              <th>Window</th>
              <th>1st-pass</th>
              <th>Median execution</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>{trend.recent10.label}</td>
              <td>{formatRate(trend.recent10.firstPass)}</td>
              <td>{formatDurationMs(trend.recent10.medianExecutionMs)}</td>
            </tr>
            <tr>
              <td>{trend.recent25.label}</td>
              <td>{formatRate(trend.recent25.firstPass)}</td>
              <td>{formatDurationMs(trend.recent25.medianExecutionMs)}</td>
            </tr>
            {weekly && (
              <tr>
                <td>
                  {weekly.previous.label} → {weekly.current.label}
                </td>
                <td>
                  {formatPercent(weekly.previous.firstPass)} →{" "}
                  {formatPercent(weekly.current.firstPass)}
                  {weekly.firstPassDeltaPp !== null &&
                    ` (${formatSignedPp(weekly.firstPassDeltaPp)})`}
                </td>
                <td>
                  {formatDurationMs(weekly.previous.medianExecutionMs)} →{" "}
                  {formatDurationMs(weekly.current.medianExecutionMs)}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <div className="performance-columns">
        <ReasonList title="Rework reasons (request-changes)" reasons={detail.reworkReasons} />
        <ReasonList title="Failure reasons" reasons={detail.failureReasons} />
      </div>
    </section>
  );
}

export function PerformanceView({ initialAgent = null }: { initialAgent?: string | null }) {
  const [window, setWindow] = useState("all");
  const [role, setRole] = useState("");
  const [operation, setOperation] = useState("");
  const [language, setLanguage] = useState("");
  const [overview, setOverview] = useState<PerformanceOverview | null>(null);
  const [error, setError] = useState("");
  const [selectedAgent, setSelectedAgent] = useState<string | null>(initialAgent);
  const [detail, setDetail] = useState<AgentPerformanceDetail | null>(null);
  const [detailError, setDetailError] = useState("");

  const filters = useCallback(
    (): PerformanceFilters => ({
      window: window === "all" ? undefined : window,
      role: role || undefined,
      operation: operation || undefined,
      language: language || undefined,
    }),
    [window, role, operation, language]
  );

  useEffect(() => {
    setError("");
    fetchPerformanceOverview(filters())
      .then(setOverview)
      .catch((reason: Error) => setError(reason.message));
  }, [filters]);

  useEffect(() => {
    if (!selectedAgent) {
      setDetail(null);
      setDetailError("");
      return;
    }
    setDetail(null);
    setDetailError("");
    fetchAgentPerformance(selectedAgent, filters())
      .then(setDetail)
      .catch((reason: Error) => setDetailError(reason.message));
  }, [selectedAgent, filters]);

  const select = (agent: string) => {
    setSelectedAgent(agent === selectedAgent ? null : agent);
  };

  const facets = overview?.facets;
  const asOptions = (values: string[], languageize = false) =>
    values.map((value) => ({
      value,
      label: languageize ? languageLabel(value) : value,
    }));

  return (
    <section className="performance-view" aria-label="Agent performance">
      <div className="performance-head">
        <div>
          <h2>Performance</h2>
          <p className="muted">
            Measured from local workflow history. Small samples are reported as insufficient data
            rather than percentages.
          </p>
        </div>
        <div className="performance-filters">
          <Select
            id="performance-window"
            label="Window"
            value={window}
            options={WINDOWS}
            onChange={setWindow}
          />
          <Select
            id="performance-role"
            label="Role"
            value={role}
            options={asOptions(facets?.roles ?? [])}
            onChange={setRole}
          />
          <Select
            id="performance-operation"
            label="Operation"
            value={operation}
            options={asOptions(facets?.operations ?? [])}
            onChange={setOperation}
          />
          <Select
            id="performance-language"
            label="Language"
            value={language}
            options={asOptions(facets?.languages ?? [], true)}
            onChange={setLanguage}
          />
        </div>
      </div>

      {error && <p className="error">{error}</p>}
      {!error && overview === null && <p className="empty-title">Loading…</p>}
      {overview !== null && (
        <OverviewTable agents={overview.agents} onSelect={select} selected={selectedAgent} />
      )}

      {selectedAgent && detailError && <p className="error">{detailError}</p>}
      {selectedAgent && !detailError && detail === null && <p className="empty-title">Loading…</p>}
      {selectedAgent && detail !== null && (
        <AgentDetail detail={detail} onClose={() => setSelectedAgent(null)} />
      )}
    </section>
  );
}
