import { useCallback, useEffect, useMemo, useState } from "react";
import {
  createPullRequest,
  fetchDelivery,
  fetchPrPreview,
  fetchRoles,
  fetchRun,
  updateWorkflowTeam,
} from "../api";
import type {
  DeliveryReport,
  GraphNode,
  PullRequestInfo,
  RoleInfo,
  RunDetail,
  Task,
  WorkflowTeam,
} from "../types";
import { isTaskIntegrated, PIPELINE_ROLE_IDS, roleAgents, runMeta } from "../types";
import { ArtifactList } from "./ArtifactList";

type WorkflowTab = "overview" | "tasks" | "activity";

const TEAM_EDITABLE_STATUSES = ["planning", "planned", "blocked"];

const DELIVERY_STATE_LABELS: Record<string, string> = {
  not_ready: "Not published",
  ready: "Ready to publish",
  pushing: "Pushing…",
  creating_pr: "Creating pull request…",
  published: "Published",
  failed: "Failed",
};

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

function teamSummaryLines(team: WorkflowTeam): string[] {
  return [
    `Planner: ${team.planner || "Not configured"}`,
    `Workers: ${team.workers.length ? team.workers.join(", ") : "Not configured"}`,
    `Reviewers: ${team.reviewers.length ? team.reviewers.join(", ") : "Not configured"}`,
    ...Object.entries(team.additional).map(([role, agents]) => `${role}: ${agents.join(", ")}`),
  ];
}

export function WorkflowInspector({
  node,
  onClose,
  onStart,
  onCancel,
  onRetry,
  onTeamUpdated,
}: {
  node: GraphNode;
  onClose: () => void;
  onStart: (runId: number) => Promise<void>;
  onCancel: (runId: number) => Promise<void>;
  onRetry: (taskId: number) => Promise<void>;
  onTeamUpdated?: () => void;
}) {
  const meta = runMeta(node);
  const [tab, setTab] = useState<WorkflowTab>("overview");
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [roles, setRoles] = useState<RoleInfo[] | null>(null);
  const [editingTeam, setEditingTeam] = useState(false);
  const [teamPlanner, setTeamPlanner] = useState("");
  const [teamWorkers, setTeamWorkers] = useState<string[]>([]);
  const [teamReviewers, setTeamReviewers] = useState<string[]>([]);
  const [teamAdditional, setTeamAdditional] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [delivery, setDelivery] = useState<DeliveryReport | null>(null);
  const [prForm, setPrForm] = useState<{ title: string; body: string; draft: boolean } | null>(
    null
  );
  const [prPreview, setPrPreview] = useState<{
    repository: string;
    base: string;
    head: string;
    existing: PullRequestInfo | null;
    eligible: boolean;
    blockers: string[];
  } | null>(null);

  const loadDetail = useCallback(() => {
    fetchRun(meta.runId)
      .then((next) => {
        setDetail(next);
        setError(null);
      })
      .catch((reason: Error) => setError(reason.message));
  }, [meta.runId]);

  const loadDelivery = useCallback(() => {
    fetchDelivery(meta.runId)
      .then((report) => setDelivery(report))
      .catch(() => setDelivery(null));
  }, [meta.runId]);

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

  const status = detail?.run.status ?? meta.status;
  const team = detail?.run.team ?? meta.team;
  const teamEditable = TEAM_EDITABLE_STATUSES.includes(status);

  // Delivery metadata is fetched on mount and whenever the workflow status
  // settles; it is persisted, so it also survives reloads.
  useEffect(() => {
    loadDelivery();
  }, [loadDelivery, status]);

  useEffect(() => {
    if (!editingTeam) return;
    if (roles !== null) return;
    fetchRoles()
      .then((next) => setRoles(next))
      .catch((reason: Error) => setError(reason.message));
  }, [editingTeam, roles]);

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
    const teamText = team ? `\n${teamSummaryLines(team).join("\n")}\n` : "";
    if (
      !window.confirm(
        `Start workflow?\n\n${count} planned task${count === 1 ? "" : "s"}\n${teamText}\nRepository changes will be made in isolated worktrees.`
      )
    ) {
      return;
    }
    void act(() => onStart(meta.runId));
  };

  const openTeamEditor = () => {
    setTeamPlanner(team?.planner ?? "");
    setTeamWorkers(team ? [...team.workers] : []);
    setTeamReviewers(team ? [...team.reviewers] : []);
    setTeamAdditional(
      team
        ? Object.fromEntries(
            Object.entries(team.additional).map(([role, agents]) => [role, [...agents]])
          )
        : {}
    );
    setEditingTeam(true);
  };

  const saveTeam = () => {
    if (!teamPlanner || teamWorkers.length === 0 || teamReviewers.length === 0) {
      setError("Pick a planner, at least one worker and at least one reviewer.");
      return;
    }
    const additional = Object.fromEntries(
      Object.entries(teamAdditional).filter(([, selected]) => selected.length > 0)
    );
    setBusy(true);
    setError(null);
    updateWorkflowTeam(meta.runId, {
      planner: teamPlanner,
      workers: teamWorkers,
      reviewers: teamReviewers,
      additional,
    })
      .then(() => {
        setEditingTeam(false);
        setBusy(false);
        loadDetail();
        onTeamUpdated?.();
      })
      .catch((reason: Error) => {
        setError((reason as Error).message);
        setBusy(false);
      });
  };

  const openPrPreview = () => {
    setBusy(true);
    setError(null);
    fetchPrPreview(meta.runId)
      .then((preview) => {
        setPrPreview({
          repository: preview.repository,
          base: preview.base,
          head: preview.head,
          existing: preview.existing,
          eligible: preview.eligible,
          blockers: preview.blockers,
        });
        setPrForm({ title: preview.title, body: preview.body, draft: preview.draft });
        setBusy(false);
      })
      .catch((reason: Error) => {
        setError((reason as Error).message);
        setBusy(false);
      });
  };

  const submitPullRequest = () => {
    if (!prForm || !prForm.title.trim()) return;
    setBusy(true);
    setError(null);
    createPullRequest(meta.runId, {
      title: prForm.title,
      body: prForm.body,
      draft: prForm.draft,
    })
      .then(() => {
        setBusy(false);
        setPrForm(null);
        setPrPreview(null);
        loadDelivery();
        onTeamUpdated?.();
      })
      .catch((reason: Error) => {
        setError((reason as Error).message);
        setBusy(false);
        loadDelivery();
      });
  };

  const roleById = useMemo(() => new Map((roles ?? []).map((role) => [role.id, role])), [roles]);
  const agentsForRole = useCallback(
    (id: string): string[] => {
      const role = roleById.get(id);
      return role ? roleAgents(role) : [];
    },
    [roleById]
  );
  const optionalTeamRoles = (roles ?? []).filter(
    (role) =>
      !PIPELINE_ROLE_IDS.includes(role.id as (typeof PIPELINE_ROLE_IDS)[number]) &&
      (role.kind === "custom" || role.available)
  );

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
              <dd>{status}</dd>
            </div>
            {(delivery?.link ?? meta.github) && (
              <div>
                <dt>Source</dt>
                <dd className="workflow-github-source">
                  GitHub Issue #{delivery?.link?.issueNumber ?? meta.github?.issueNumber} —{" "}
                  {delivery?.link?.issueTitle ?? meta.github?.issueTitle}
                  {delivery?.link?.issueUrl || meta.github?.issueUrl ? (
                    <a
                      href={delivery?.link?.issueUrl ?? meta.github?.issueUrl}
                      target="_blank"
                      rel="noreferrer"
                    >
                      Open on GitHub
                    </a>
                  ) : null}
                </dd>
              </div>
            )}
            <div>
              <dt>Team</dt>
              <dd className="workflow-team-summary">
                {team ? (
                  <ul>
                    <li>
                      <span>Planner</span>
                      <strong>{team.planner || "Not configured"}</strong>
                    </li>
                    <li>
                      <span>Workers</span>
                      <strong>{team.workers.length ? team.workers.join(", ") : "None"}</strong>
                    </li>
                    <li>
                      <span>Reviewers</span>
                      <strong>{team.reviewers.length ? team.reviewers.join(", ") : "None"}</strong>
                    </li>
                    {Object.entries(team.additional).map(([role, agents]) => (
                      <li key={role}>
                        <span>{role}</span>
                        <strong>{agents.join(", ")}</strong>
                      </li>
                    ))}
                  </ul>
                ) : (
                  "Not configured"
                )}
              </dd>
            </div>
            <div>
              <dt>Progress</dt>
              <dd>{progressLabel(detail, meta)} tasks</dd>
            </div>
            {detail && detail.integration && (
              <div>
                <dt>Integration</dt>
                <dd>
                  <ul className="workflow-integration-summary">
                    <li>
                      <span>Branch</span>
                      <strong>{detail.integration.branch}</strong>
                    </li>
                    <li>
                      <span>Head</span>
                      <strong>
                        {detail.integration.head
                          ? detail.integration.head.slice(0, 12)
                          : "No commits yet"}
                      </strong>
                    </li>
                    <li>
                      <span>Integrated</span>
                      <strong>
                        {detail.integration.integratedTasks.length} task
                        {detail.integration.integratedTasks.length === 1 ? "" : "s"}
                      </strong>
                    </li>
                  </ul>
                </dd>
              </div>
            )}
            <div>
              <dt>Objective</dt>
              <dd>{meta.objective}</dd>
            </div>
          </dl>

          {delivery && (delivery.link || status === "completed") && (
            <section className="workflow-delivery" aria-label="GitHub delivery">
              <h4>Delivery</h4>
              <ul className="workflow-integration-summary">
                {delivery.link && (
                  <li>
                    <span>GitHub Issue</span>
                    <strong>#{delivery.link.issueNumber}</strong>
                  </li>
                )}
                <li>
                  <span>Integration</span>
                  <strong>
                    {delivery.headBranch}
                    {delivery.integrationHead ? " ✓" : ""}
                  </strong>
                </li>
                <li>
                  <span>Pull Request</span>
                  <strong>
                    {delivery.pullRequest ? (
                      <>
                        #{delivery.pullRequest.number}{" "}
                        {delivery.pullRequest.isDraft ? "Draft" : delivery.pullRequest.state}
                      </>
                    ) : (
                      (DELIVERY_STATE_LABELS[delivery.state] ?? delivery.state)
                    )}
                  </strong>
                </li>
              </ul>
              {delivery.repository && (
                <p className="workflow-delivery-repo">
                  {delivery.repository.repository} · base {delivery.baseBranch ?? "main"}
                </p>
              )}
              {delivery.error && <p className="inline-error">{delivery.error}</p>}
              {!delivery.pullRequest && delivery.blockers.length > 0 && status === "completed" && (
                <ul className="workflow-delivery-blockers">
                  {delivery.blockers.map((blocker) => (
                    <li key={blocker}>{blocker}</li>
                  ))}
                </ul>
              )}
              {prForm && prPreview ? (
                <div className="workflow-pr-form">
                  <h4>Create Pull Request</h4>
                  <dl className="inspector-list">
                    <div>
                      <dt>Repository</dt>
                      <dd>{prPreview.repository}</dd>
                    </div>
                    <div>
                      <dt>Base</dt>
                      <dd>{prPreview.base}</dd>
                    </div>
                    <div>
                      <dt>Head</dt>
                      <dd>{prPreview.head}</dd>
                    </div>
                  </dl>
                  {prPreview.existing && (
                    <p className="inline-note">
                      Pull request #{prPreview.existing.number} already exists for this branch and
                      will be linked instead of duplicated.
                    </p>
                  )}
                  <label>
                    <span>Title</span>
                    <input
                      value={prForm.title}
                      onChange={(event) => setPrForm({ ...prForm, title: event.target.value })}
                      autoFocus
                    />
                  </label>
                  <label>
                    <span>Body</span>
                    <textarea
                      rows={10}
                      value={prForm.body}
                      onChange={(event) => setPrForm({ ...prForm, body: event.target.value })}
                    />
                  </label>
                  <label className="workflow-team-check">
                    <input
                      type="checkbox"
                      checked={prForm.draft}
                      onChange={(event) => setPrForm({ ...prForm, draft: event.target.checked })}
                    />
                    <span>Create as draft</span>
                  </label>
                  {prPreview.blockers.length > 0 && (
                    <ul className="workflow-delivery-blockers">
                      {prPreview.blockers.map((blocker) => (
                        <li key={blocker}>{blocker}</li>
                      ))}
                    </ul>
                  )}
                  <div className="workflow-actions">
                    <button
                      className="button button-primary"
                      onClick={submitPullRequest}
                      disabled={busy || !prForm.title.trim() || !prPreview.eligible}
                    >
                      Create Pull Request
                    </button>
                    <button
                      className="button"
                      onClick={() => {
                        setPrForm(null);
                        setPrPreview(null);
                      }}
                      disabled={busy}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ) : (
                <div className="workflow-actions">
                  {delivery.pullRequest?.url && (
                    <a
                      className="button"
                      href={delivery.pullRequest.url}
                      target="_blank"
                      rel="noreferrer"
                    >
                      Open on GitHub
                    </a>
                  )}
                  {!delivery.pullRequest && delivery.eligible && (
                    <button
                      className="button button-primary"
                      onClick={openPrPreview}
                      disabled={busy}
                    >
                      Create Pull Request
                    </button>
                  )}
                </div>
              )}
            </section>
          )}

          {detail && detail.stages.length > 0 && (
            <section className="workflow-stages" aria-label="Workflow stages">
              <h4>Stages</h4>
              <ul>
                {detail.stages.map((stage) => (
                  <li key={stage.key} className={`stage--${stage.state}`}>
                    <span>{stage.label}</span>
                    <strong>{stage.state === "completed" ? "✓" : stage.state}</strong>
                    <small>
                      {stage.completed}/{stage.total}
                    </small>
                  </li>
                ))}
              </ul>
            </section>
          )}

          {detail && detail.artifacts.length > 0 && (
            <details className="agent-advanced workflow-artifacts">
              <summary>Role artifacts ({detail.artifacts.length})</summary>
              <ArtifactList artifacts={detail.artifacts} />
            </details>
          )}

          {editingTeam ? (
            <div className="workflow-team-editor">
              {roles === null ? (
                <p className="inspector-hint">Loading role assignments…</p>
              ) : (
                <>
                  <label>
                    <span>Planner</span>
                    <select
                      value={teamPlanner}
                      onChange={(event) => setTeamPlanner(event.target.value)}
                    >
                      <option value="">Select a planner</option>
                      {agentsForRole("planner").map((agent) => (
                        <option key={agent} value={agent}>
                          {agent}
                        </option>
                      ))}
                    </select>
                  </label>
                  <fieldset className="workflow-team-group">
                    <legend>Workers</legend>
                    {agentsForRole("worker").map((agent) => (
                      <label key={agent} className="workflow-team-check">
                        <input
                          type="checkbox"
                          checked={teamWorkers.includes(agent)}
                          onChange={(event) =>
                            setTeamWorkers((current) =>
                              event.target.checked
                                ? [...current, agent]
                                : current.filter((candidate) => candidate !== agent)
                            )
                          }
                        />
                        <span>{agent}</span>
                      </label>
                    ))}
                  </fieldset>
                  <fieldset className="workflow-team-group">
                    <legend>Reviewers</legend>
                    {agentsForRole("reviewer").map((agent) => (
                      <label key={agent} className="workflow-team-check">
                        <input
                          type="checkbox"
                          checked={teamReviewers.includes(agent)}
                          onChange={(event) =>
                            setTeamReviewers((current) =>
                              event.target.checked
                                ? [...current, agent]
                                : current.filter((candidate) => candidate !== agent)
                            )
                          }
                        />
                        <span>{agent}</span>
                      </label>
                    ))}
                  </fieldset>
                  {optionalTeamRoles.length > 0 && (
                    <details className="agent-advanced">
                      <summary>Additional roles</summary>
                      {optionalTeamRoles.map((role) => (
                        <fieldset key={role.id} className="workflow-team-group">
                          <legend>{role.name}</legend>
                          {role.assignments.map((assignment) => (
                            <label key={assignment.agent} className="workflow-team-check">
                              <input
                                type="checkbox"
                                checked={(teamAdditional[role.id] ?? []).includes(assignment.agent)}
                                onChange={(event) =>
                                  setTeamAdditional((current) => {
                                    const selected = current[role.id] ?? [];
                                    return {
                                      ...current,
                                      [role.id]: event.target.checked
                                        ? [...selected, assignment.agent]
                                        : selected.filter(
                                            (candidate) => candidate !== assignment.agent
                                          ),
                                    };
                                  })
                                }
                              />
                              <span>{assignment.agent}</span>
                            </label>
                          ))}
                        </fieldset>
                      ))}
                    </details>
                  )}
                  <div className="workflow-actions">
                    <button className="button" onClick={saveTeam} disabled={busy}>
                      Save team
                    </button>
                    <button className="button" onClick={() => setEditingTeam(false)}>
                      Cancel
                    </button>
                  </div>
                </>
              )}
            </div>
          ) : (
            <div className="workflow-actions">
              {status === "planned" && (
                <button className="button button-primary" onClick={start} disabled={busy}>
                  Start
                </button>
              )}
              {teamEditable && (
                <button className="button" onClick={openTeamEditor} disabled={busy}>
                  Edit team
                </button>
              )}
              {status === "active" && (
                <p className="inline-note">The team is locked while the workflow is active.</p>
              )}
              {["planning", "active"].includes(status) && (
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
          )}
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
                <p className="workflow-task-meta">
                  <span className="op-chip">{task.operation ?? "implement"}</span>
                  <span>{task.role ?? "worker"}</span>
                  {isTaskIntegrated(task, detail.integration) && (
                    <span
                      className="integrated-tag"
                      title="Approved work merged into the integration branch"
                    >
                      ✓ integrated
                    </span>
                  )}
                </p>
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
