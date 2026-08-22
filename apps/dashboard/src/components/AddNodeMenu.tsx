import { useEffect, useRef, useState } from "react";
import { fetchGithubStatus } from "../api";
import type {
  AgentEntry,
  AgentKind,
  ConfigData,
  GitHubStatus,
  PromptTransport,
  RoleInfo,
  WorkflowTeam,
} from "../types";
import { PIPELINE_ROLE_IDS, preferredRoleAgents, roleAgents } from "../types";
import { RoleForm, type RoleFormValue } from "./RoleForm";

type AddKind = "workflow" | "agent" | "role" | "group" | "note";
type RoleMode = "core" | "custom";
type WorkflowSource = "objective" | "github";
const AGENT_PRESETS: Record<
  Exclude<AgentKind, "custom">,
  { label: string; name: string; command: string; args: string[] }
> = {
  codex: { label: "Codex", name: "codex", command: "codex", args: ["exec"] },
  claude_code: { label: "Claude Code", name: "claude", command: "claude", args: ["-p"] },
  open_code: { label: "OpenCode", name: "opencode", command: "opencode", args: ["run"] },
  gemini_cli: { label: "Gemini CLI", name: "gemini", command: "gemini", args: ["-p"] },
  qwen_code: { label: "Qwen Code", name: "qwen", command: "qwen", args: ["-p"] },
};

function parseLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

function parseEnvironment(text: string): {
  environment: Record<string, string>;
  error: string | null;
} {
  const environment: Record<string, string> = {};
  for (const [index, rawLine] of text.split("\n").entries()) {
    const line = rawLine.trim();
    if (!line) continue;
    const separator = line.indexOf("=");
    const key = line.slice(0, Math.max(separator, 0)).trim();
    if (separator <= 0 || !key) {
      return {
        environment: {},
        error: `Environment line ${index + 1} must use KEY=VALUE.`,
      };
    }
    environment[key] = line.slice(separator + 1).trim();
  }
  return { environment, error: null };
}

export function AddNodeMenu({
  open,
  config,
  roles,
  error,
  initialKind,
  onClose,
  onCreateWorkflow,
  onCreateWorkflowFromIssue,
  onCreateAgent,
  onCreateRole,
  onAssignCoreRole,
  onCreateVisual,
}: {
  open: boolean;
  config: ConfigData;
  roles: RoleInfo[];
  error: string | null;
  initialKind?: AddKind | null;
  onClose: () => void;
  onCreateWorkflow: (objective: string, team: WorkflowTeam) => void;
  onCreateWorkflowFromIssue: (issue: string, team: WorkflowTeam) => void;
  onCreateAgent: (name: string, entry: AgentEntry) => void;
  onCreateRole: (value: RoleFormValue) => void;
  onAssignCoreRole: (roleId: string, agent: string) => void;
  onCreateVisual: (kind: "group" | "note", label: string, text: string) => void;
}) {
  const [kind, setKind] = useState<AddKind | null>(null);
  const [name, setName] = useState("");
  const [agentKind, setAgentKind] = useState<AgentKind>("codex");
  const [command, setCommand] = useState("codex");
  const [argumentsText, setArgumentsText] = useState("exec");
  const [environmentText, setEnvironmentText] = useState("");
  const [promptTransport, setPromptTransport] = useState<PromptTransport>("stdin");
  const [interactive, setInteractive] = useState(false);
  const [interactiveArgumentsText, setInteractiveArgumentsText] = useState("");
  const [roleMode, setRoleMode] = useState<RoleMode | null>(null);
  const [coreRoleDraft, setCoreRoleDraft] = useState<RoleInfo | null>(null);
  const [coreRoleAgent, setCoreRoleAgent] = useState("");
  const [text, setText] = useState("");
  const [objective, setObjective] = useState("");
  const [workflowSource, setWorkflowSource] = useState<WorkflowSource>("objective");
  const [issueReference, setIssueReference] = useState("");
  const [githubStatus, setGithubStatus] = useState<GitHubStatus | null>(null);
  const [teamPlanner, setTeamPlanner] = useState("");
  const [teamWorkers, setTeamWorkers] = useState<string[]>([]);
  const [teamReviewers, setTeamReviewers] = useState<string[]>([]);
  const [teamAdditional, setTeamAdditional] = useState<Record<string, string[]>>({});
  const [validationError, setValidationError] = useState<string | null>(null);
  const firstButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      setKind(initialKind ?? null);
      window.requestAnimationFrame(() => firstButton.current?.focus());
    } else {
      setKind(null);
      setRoleMode(null);
      setCoreRoleDraft(null);
      setValidationError(null);
    }
  }, [initialKind, open]);

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose, open]);

  useEffect(() => {
    if (!open || kind !== "workflow") return;
    const byId = new Map(roles.map((role) => [role.id, role]));
    setTeamPlanner(preferredRoleAgents(byId.get("planner"))[0] ?? "");
    setTeamWorkers(preferredRoleAgents(byId.get("worker")));
    setTeamReviewers(preferredRoleAgents(byId.get("reviewer")));
    setTeamAdditional({});
  }, [kind, open, roles]);

  useEffect(() => {
    if (!open || kind !== "workflow" || workflowSource !== "github" || githubStatus) return;
    fetchGithubStatus()
      .then((status) => setGithubStatus(status))
      .catch((reason: Error) =>
        setGithubStatus({
          connected: false,
          user: null,
          authError: (reason as Error).message,
          remoteError: null,
          repository: null,
        })
      );
  }, [githubStatus, kind, open, workflowSource]);

  useEffect(() => {
    if (!open) {
      setGithubStatus(null);
      setWorkflowSource("objective");
      setIssueReference("");
    }
  }, [open]);

  if (!open) return null;
  const agents = Object.keys(config.agents).sort();
  const roleById = new Map(roles.map((role) => [role.id, role]));
  const plannerRole = roleById.get("planner");
  const plannerAgents = plannerRole ? roleAgents(plannerRole) : [];
  const workerRole = roleById.get("worker");
  const workerAgents = workerRole ? roleAgents(workerRole) : [];
  const reviewerRole = roleById.get("reviewer");
  const reviewerAgents = reviewerRole ? roleAgents(reviewerRole) : [];
  const optionalRoles = roles.filter(
    (role) =>
      !PIPELINE_ROLE_IDS.includes(role.id as (typeof PIPELINE_ROLE_IDS)[number]) &&
      (role.kind === "custom" || role.available)
  );
  const dormantCoreRoles = roles.filter(
    (role) =>
      role.kind === "core" &&
      !PIPELINE_ROLE_IDS.includes(role.id as (typeof PIPELINE_ROLE_IDS)[number]) &&
      !role.available
  );
  const workflowReady = teamPlanner !== "" && teamWorkers.length > 0 && teamReviewers.length > 0;

  const toggleTeamAgent = (
    selection: string[],
    setSelection: (next: string[]) => void,
    agent: string,
    checked: boolean
  ) => {
    setSelection(
      checked ? [...selection, agent] : selection.filter((candidate) => candidate !== agent)
    );
  };

  const toggleAdditionalAgent = (roleId: string, agent: string, checked: boolean) => {
    const current = teamAdditional[roleId] ?? [];
    const next = checked ? [...current, agent] : current.filter((candidate) => candidate !== agent);
    setTeamAdditional({ ...teamAdditional, [roleId]: next });
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (kind === "workflow") {
      if (workflowSource === "objective" && !objective.trim()) {
        setValidationError("Describe what the Factory should build.");
        return;
      }
      if (workflowSource === "github" && !issueReference.trim()) {
        setValidationError("Provide an issue number (#42) or a GitHub issue URL.");
        return;
      }
      if (!workflowReady) {
        setValidationError("Pick a planner, at least one worker and at least one reviewer.");
        return;
      }
      setValidationError(null);
      const additional = Object.fromEntries(
        Object.entries(teamAdditional).filter(([, selected]) => selected.length > 0)
      );
      const team: WorkflowTeam = {
        planner: teamPlanner,
        workers: teamWorkers,
        reviewers: teamReviewers,
        additional,
      };
      if (workflowSource === "github") {
        onCreateWorkflowFromIssue(issueReference.trim(), team);
      } else {
        onCreateWorkflow(objective.trim(), team);
      }
    } else if (kind === "agent") {
      const parsedEnvironment = parseEnvironment(environmentText);
      if (parsedEnvironment.error) {
        setValidationError(parsedEnvironment.error);
        return;
      }
      setValidationError(null);
      const entry: AgentEntry = {
        kind: agentKind,
        command: command.trim(),
        args: parseLines(argumentsText),
        env: parsedEnvironment.environment,
        capabilities: [],
      };
      if (agentKind === "custom") {
        entry.prompt_transport = promptTransport;
        if (interactive) entry.interactive_args = parseLines(interactiveArgumentsText);
      }
      onCreateAgent(name.trim(), entry);
    } else if (kind === "role") {
      if (coreRoleDraft && coreRoleAgent) onAssignCoreRole(coreRoleDraft.id, coreRoleAgent);
    } else if (kind) {
      onCreateVisual(kind, name.trim(), text.trim());
    }
  };

  const submitLabel = coreRoleDraft ? "Assign" : kind === "workflow" ? "Plan" : "Create";

  return (
    <div className="add-node-menu" role="dialog" aria-label="Add graph node">
      <div className="add-node-menu-head">
        <strong>
          {kind ? `New ${kind}` : "Add node"}
          {kind === "role" && roleMode ? ` — ${roleMode}` : ""}
        </strong>
        <button className="inspector-close" onClick={onClose} aria-label="Close add node menu">
          x
        </button>
      </div>
      {kind === null ? (
        <div className="add-node-types">
          {(["workflow", "agent", "role", "group", "note"] as AddKind[]).map((option, index) => (
            <button
              key={option}
              ref={index === 0 ? firstButton : undefined}
              className="add-node-type"
              onClick={() => setKind(option)}
            >
              <strong>{option[0].toUpperCase() + option.slice(1)}</strong>
              <span>
                {option === "agent"
                  ? "Configured external process"
                  : option === "workflow"
                    ? "Plan work with a Factory team"
                    : option === "role"
                      ? "Core assignment or custom role"
                      : option === "group"
                        ? "Visual organization"
                        : "Workspace context"}
              </span>
            </button>
          ))}
        </div>
      ) : kind === "role" && roleMode === "custom" ? (
        <RoleForm
          mode="create"
          agents={agents}
          error={error}
          submitLabel="Create role"
          onSubmit={onCreateRole}
          onCancel={() => setRoleMode(null)}
        />
      ) : (
        <form className="add-node-form" onSubmit={submit}>
          {kind === "workflow" && (
            <>
              <div className="workflow-source" role="tablist" aria-label="Workflow source">
                <button
                  type="button"
                  role="tab"
                  aria-selected={workflowSource === "objective"}
                  className={workflowSource === "objective" ? "is-active" : ""}
                  onClick={() => setWorkflowSource("objective")}
                >
                  From objective
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={workflowSource === "github"}
                  className={workflowSource === "github" ? "is-active" : ""}
                  onClick={() => setWorkflowSource("github")}
                >
                  From GitHub Issue
                </button>
              </div>
              {workflowSource === "objective" ? (
                <label>
                  <span>What should the Factory build?</span>
                  <textarea
                    rows={5}
                    value={objective}
                    onChange={(event) => {
                      setObjective(event.target.value);
                      setValidationError(null);
                    }}
                    placeholder="Implement authentication with email login and password reset."
                    required
                    autoFocus
                  />
                </label>
              ) : (
                <>
                  <label>
                    <span>GitHub Issue</span>
                    <input
                      value={issueReference}
                      onChange={(event) => {
                        setIssueReference(event.target.value);
                        setValidationError(null);
                      }}
                      placeholder="#42 or https://github.com/owner/repo/issues/42"
                      autoFocus
                    />
                  </label>
                  {githubStatus === null ? (
                    <p className="inline-note">Checking the GitHub CLI…</p>
                  ) : githubStatus.connected && githubStatus.repository ? (
                    <p className="inline-note">
                      GitHub — connected as {githubStatus.user ?? "unknown user"} ·{" "}
                      {githubStatus.repository.repository}
                    </p>
                  ) : (
                    <p className="inline-note">
                      {githubStatus.authError ??
                        githubStatus.remoteError ??
                        "GitHub is not connected."}
                    </p>
                  )}
                  <p className="inline-note">
                    The issue body is imported as untrusted context; the workflow still needs an
                    explicit Start before anything runs.
                  </p>
                </>
              )}
              <div className="workflow-team">
                <span className="workflow-team-title">Team</span>
                <label>
                  <span>Planner</span>
                  <select
                    value={teamPlanner}
                    onChange={(event) => setTeamPlanner(event.target.value)}
                  >
                    <option value="">Select a planner</option>
                    {plannerAgents.map((agent) => (
                      <option key={agent} value={agent}>
                        {agent}
                        {plannerRole?.assignments.some(
                          (assignment) => assignment.agent === agent && assignment.preferred
                        )
                          ? " (preferred)"
                          : ""}
                      </option>
                    ))}
                  </select>
                </label>
                {plannerAgents.length === 0 && (
                  <p className="inline-note">
                    No planner configured. Assign an agent to the planner role before creating a
                    workflow.
                  </p>
                )}
                <fieldset className="workflow-team-group">
                  <legend>Workers</legend>
                  {workerAgents.length === 0 ? (
                    <p className="inline-note">No worker configured.</p>
                  ) : (
                    workerAgents.map((agent) => (
                      <label key={agent} className="workflow-team-check">
                        <input
                          type="checkbox"
                          checked={teamWorkers.includes(agent)}
                          onChange={(event) =>
                            toggleTeamAgent(
                              teamWorkers,
                              setTeamWorkers,
                              agent,
                              event.target.checked
                            )
                          }
                        />
                        <span>{agent}</span>
                      </label>
                    ))
                  )}
                </fieldset>
                <fieldset className="workflow-team-group">
                  <legend>Reviewers</legend>
                  {reviewerAgents.length === 0 ? (
                    <p className="inline-note">No reviewer configured.</p>
                  ) : (
                    reviewerAgents.map((agent) => (
                      <label key={agent} className="workflow-team-check">
                        <input
                          type="checkbox"
                          checked={teamReviewers.includes(agent)}
                          onChange={(event) =>
                            toggleTeamAgent(
                              teamReviewers,
                              setTeamReviewers,
                              agent,
                              event.target.checked
                            )
                          }
                        />
                        <span>{agent}</span>
                      </label>
                    ))
                  )}
                </fieldset>
                <details className="agent-advanced">
                  <summary>Advanced team</summary>
                  {optionalRoles.length === 0 ? (
                    <p className="inline-note">
                      No optional roles available. Assign agents to optional core roles or create a
                      custom role.
                    </p>
                  ) : (
                    optionalRoles.map((role) => (
                      <fieldset key={role.id} className="workflow-team-group">
                        <legend>{role.name}</legend>
                        {(teamAdditional[role.id] ?? []).length === 0 && (
                          <p className="inline-note">
                            Optional — skipped unless an agent is picked.
                          </p>
                        )}
                        {role.assignments.map((assignment) => (
                          <label key={assignment.agent} className="workflow-team-check">
                            <input
                              type="checkbox"
                              checked={(teamAdditional[role.id] ?? []).includes(assignment.agent)}
                              onChange={(event) =>
                                toggleAdditionalAgent(
                                  role.id,
                                  assignment.agent,
                                  event.target.checked
                                )
                              }
                            />
                            <span>{assignment.agent}</span>
                          </label>
                        ))}
                      </fieldset>
                    ))
                  )}
                </details>
              </div>
            </>
          )}
          {kind === "agent" && (
            <>
              <label>
                <span>Name</span>
                <input value={name} onChange={(event) => setName(event.target.value)} required />
              </label>
              <label>
                <span>Agent type</span>
                <select
                  value={agentKind}
                  onChange={(event) => {
                    const next = event.target.value as AgentKind;
                    setAgentKind(next);
                    if (next === "custom") {
                      setCommand("");
                      setArgumentsText("");
                      return;
                    }
                    const preset = AGENT_PRESETS[next];
                    setName((current) => current || preset.name);
                    setCommand(preset.command);
                    setArgumentsText(preset.args.join("\n"));
                  }}
                >
                  {Object.entries(AGENT_PRESETS).map(([value, preset]) => (
                    <option key={value} value={value}>
                      {preset.label}
                    </option>
                  ))}
                  <option value="custom">Custom</option>
                </select>
              </label>
              <details className="agent-advanced" open={agentKind === "custom" || undefined}>
                <summary>{agentKind === "custom" ? "Invocation" : "Advanced"}</summary>
                <label>
                  <span>Command</span>
                  <input
                    value={command}
                    onChange={(event) => setCommand(event.target.value)}
                    required
                  />
                </label>
                <label>
                  <span>Workflow arguments</span>
                  <textarea
                    rows={3}
                    value={argumentsText}
                    onChange={(event) => setArgumentsText(event.target.value)}
                    placeholder="One argument per line"
                  />
                </label>
                {agentKind === "custom" && (
                  <>
                    <label>
                      <span>Workflow prompt transport</span>
                      <select
                        value={promptTransport}
                        onChange={(event) =>
                          setPromptTransport(event.target.value as PromptTransport)
                        }
                      >
                        <option value="stdin">Pass mission through stdin</option>
                        <option value="argument">Pass mission as an argument</option>
                        <option value="disabled">Interactive sessions only</option>
                      </select>
                    </label>
                    {promptTransport === "argument" && (
                      <p className="inline-note">
                        Use a single <code>{"{mission}"}</code> argument to control placement, or
                        omit it to append the mission.
                      </p>
                    )}
                    <label className="agent-interactive-toggle">
                      <input
                        type="checkbox"
                        checked={interactive}
                        onChange={(event) => setInteractive(event.target.checked)}
                      />
                      <span>Enable an interactive console invocation</span>
                    </label>
                    {interactive && (
                      <label>
                        <span>Interactive arguments</span>
                        <textarea
                          rows={2}
                          value={interactiveArgumentsText}
                          onChange={(event) => setInteractiveArgumentsText(event.target.value)}
                          placeholder="One argument per line"
                        />
                      </label>
                    )}
                  </>
                )}
                <label>
                  <span>Environment</span>
                  <textarea
                    rows={3}
                    value={environmentText}
                    onChange={(event) => {
                      setEnvironmentText(event.target.value);
                      setValidationError(null);
                    }}
                    placeholder="KEY=VALUE, one per line"
                  />
                </label>
              </details>
            </>
          )}
          {kind === "role" && roleMode === null && (
            <div className="add-node-types">
              <button
                ref={firstButton}
                className="add-node-type"
                onClick={() => setRoleMode("core")}
              >
                <strong>Core role</strong>
                <span>Assign an agent to a built-in optional role</span>
              </button>
              <button className="add-node-type" onClick={() => setRoleMode("custom")}>
                <strong>Custom role</strong>
                <span>Define a new role for your factory</span>
              </button>
            </div>
          )}
          {kind === "role" && roleMode === "core" && (
            <>
              {coreRoleDraft ? (
                <label>
                  <span>Agent for {coreRoleDraft.name}</span>
                  <select
                    value={coreRoleAgent}
                    onChange={(event) => setCoreRoleAgent(event.target.value)}
                    required
                    autoFocus
                  >
                    <option value="">Select an agent</option>
                    {agents.map((option) => (
                      <option key={option} value={option}>
                        {option}
                      </option>
                    ))}
                  </select>
                </label>
              ) : dormantCoreRoles.length === 0 ? (
                <p className="inline-note">
                  Every optional core role already has an agent assigned.
                </p>
              ) : (
                <div className="add-node-types">
                  {dormantCoreRoles.map((role) => (
                    <button
                      key={role.id}
                      className="add-node-type"
                      onClick={() => setCoreRoleDraft(role)}
                    >
                      <strong>{role.name}</strong>
                      <span>{role.description}</span>
                    </button>
                  ))}
                </div>
              )}
            </>
          )}
          {(kind === "group" || kind === "note") && (
            <>
              <label>
                <span>{kind === "group" ? "Name" : "Title"}</span>
                <input value={name} onChange={(event) => setName(event.target.value)} required />
              </label>
              {kind === "note" && (
                <label>
                  <span>Text</span>
                  <textarea
                    rows={4}
                    value={text}
                    onChange={(event) => setText(event.target.value)}
                  />
                </label>
              )}
            </>
          )}
          {(validationError ?? error) && !(kind === "role" && roleMode === "custom") && (
            <p className="inline-error">{validationError ?? error}</p>
          )}
          <div className="add-node-actions">
            {!(kind === "role" && roleMode === null) && (
              <button
                className="button"
                type="submit"
                disabled={
                  (kind === "workflow" && !workflowReady) ||
                  (kind === "role" && (!coreRoleDraft || !coreRoleAgent))
                }
              >
                {submitLabel}
              </button>
            )}
            <button
              className="button"
              type="button"
              onClick={() => {
                if (kind === "role" && roleMode !== null && !coreRoleDraft) setRoleMode(null);
                else if (coreRoleDraft) setCoreRoleDraft(null);
                else setKind(null);
              }}
            >
              Back
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
