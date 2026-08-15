import { useCallback, useEffect, useState } from "react";
import {
  addRoleAssignment,
  deleteRole,
  fetchAgents,
  fetchConfig,
  fetchRoles,
  removeRoleAssignment,
  saveConfig,
  setPreferredAssignment,
  updateRole,
} from "../api";
import {
  agentResolutionStatusLabel,
  type AgentKind,
  type AgentStatusInfo,
  type ConfigData,
  type PromptTransport,
  type RoleInfo,
} from "../types";
import { RoleForm } from "./RoleForm";

const PRESETS: Record<
  Exclude<AgentKind, "custom">,
  { label: string; command: string; args: string[] }
> = {
  codex: { label: "Codex", command: "codex", args: ["exec"] },
  claude_code: { label: "Claude Code", command: "claude", args: ["-p"] },
  open_code: { label: "OpenCode", command: "opencode", args: ["run"] },
  gemini_cli: { label: "Gemini CLI", command: "gemini", args: ["-p"] },
  qwen_code: { label: "Qwen Code", command: "qwen", args: ["-p"] },
};

interface Draft {
  originalName: string | null;
  name: string;
  kind: AgentKind;
  command: string;
  argsText: string;
  envText: string;
  promptTransport: PromptTransport;
  interactive: boolean;
  interactiveArgsText: string;
}

function emptyDraft(): Draft {
  return {
    originalName: null,
    name: "",
    kind: "codex",
    command: "codex",
    argsText: "exec",
    envText: "",
    promptTransport: "stdin",
    interactive: true,
    interactiveArgsText: "",
  };
}

function inferredKind(entry: ConfigData["agents"][string]): AgentKind {
  if (entry.kind) return entry.kind;
  if (entry.command === "codex" && entry.args[0] === "exec") return "codex";
  if (entry.command === "opencode" && entry.args[0] === "run") return "open_code";
  if (entry.command === "claude" && entry.args.some((arg) => arg === "-p" || arg === "--print"))
    return "claude_code";
  if (entry.command === "gemini" && entry.args.some((arg) => arg === "-p" || arg === "--prompt"))
    return "gemini_cli";
  if (entry.command === "qwen" && entry.args.some((arg) => arg === "-p" || arg === "--prompt"))
    return "qwen_code";
  return "custom";
}

function draftFrom(name: string, entry: ConfigData["agents"][string]): Draft {
  return {
    originalName: name,
    name,
    kind: inferredKind(entry),
    command: entry.command,
    argsText: entry.args.join("\n"),
    envText: Object.entries(entry.env)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    promptTransport: entry.prompt_transport ?? "stdin",
    interactive: entry.interactive_args !== undefined || inferredKind(entry) !== "custom",
    interactiveArgsText: (entry.interactive_args ?? []).join("\n"),
  };
}

function parseLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function parseEnv(text: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    result[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
  }
  return result;
}

export function SettingsView() {
  const [config, setConfig] = useState<ConfigData | null>(null);
  const [available, setAvailable] = useState<Record<string, AgentStatusInfo>>({});
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [editingRole, setEditingRole] = useState<RoleInfo | null>(null);
  const [roleSelection, setRoleSelection] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const reload = useCallback(() => {
    setError(null);
    setSaved(null);
    Promise.all([fetchConfig(), fetchAgents(), fetchRoles()])
      .then(([nextConfig, list, nextRoles]) => {
        setConfig(nextConfig);
        setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent])));
        setRoles(nextRoles);
        setEditingRole((current) =>
          current ? (nextRoles.find((role) => role.id === current.id) ?? null) : null
        );
      })
      .catch((err: Error) => setError(err.message));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  const runRoleOperation = useCallback(
    (operation: () => Promise<unknown>, note: string) => {
      setError(null);
      setSaved(null);
      operation()
        .then(() => {
          setSaved(note);
          reload();
        })
        .catch((err: Error) => setError(err.message));
    },
    [reload]
  );

  const apply = useCallback(
    (next: ConfigData, note: string) => {
      setError(null);
      setSaved(null);
      saveConfig(next)
        .then(() => {
          setSaved(note);
          reload();
        })
        .catch((err: Error) => setError(err.message));
    },
    [reload]
  );

  const saveDraft = () => {
    if (!config || !draft) return;
    const name = draft.name.trim();
    if (!name) {
      setError("Agent name is required.");
      return;
    }
    if (!draft.command.trim()) {
      setError("Command is required.");
      return;
    }
    const agents = { ...config.agents };
    if (draft.originalName) {
      delete agents[draft.originalName];
    }
    const entry: ConfigData["agents"][string] = {
      kind: draft.kind,
      command: draft.command.trim(),
      args: parseLines(draft.argsText),
      env: parseEnv(draft.envText),
      capabilities: draft.originalName ? config.agents[draft.originalName]?.capabilities : [],
    };
    if (draft.kind === "custom") {
      entry.prompt_transport = draft.promptTransport;
      if (draft.interactive) entry.interactive_args = parseLines(draft.interactiveArgsText);
    }
    agents[name] = entry;
    const editing = draft.originalName !== null;
    setDraft(null);
    apply({ ...config, agents }, editing ? `Saved agent ${name}.` : `Added agent ${name}.`);
  };

  const removeAgent = (name: string) => {
    if (!config) return;
    const agents = { ...config.agents };
    delete agents[name];
    const role_assignments = config.role_assignments.filter((entry) => entry.agent !== name);
    setDraft(null);
    apply({ ...config, agents, role_assignments }, `Removed agent ${name}.`);
  };

  const refreshAvailability = () => {
    fetchAgents()
      .then((list) => setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent]))))
      .catch((err: Error) => setError(err.message));
  };

  if (config === null) {
    return (
      <div className="settings">
        {error ? (
          <div className="empty" role="alert">
            <p className="empty-title">Could not load Settings.</p>
            <p className="error">{error}</p>
            <div className="empty-actions">
              <button className="button" onClick={reload}>
                Retry
              </button>
            </div>
          </div>
        ) : (
          <p className="empty-title">Loading…</p>
        )}
      </div>
    );
  }

  const agentNames = Object.keys(config.agents).sort();

  return (
    <div className="settings">
      <div className="settings-header">
        <h2 className="settings-title">Settings</h2>
      </div>
      {error && <p className="error">{error}</p>}
      {saved && <p className="settings-saved">{saved}</p>}

      <section className="section">
        <h3 className="section-title">Roles</h3>
        <p className="settings-note">
          Assign configured agents to each role. Core roles are built in; custom roles can be edited
          or deleted. The config is written to <code>.factory/config.toml</code>.
        </p>
        <div className="settings-roles">
          {roles.map((role) => {
            const assigned = new Set(role.assignments.map((assignment) => assignment.agent));
            const candidates = agentNames.filter((name) => !assigned.has(name));
            const selection = roleSelection[role.id] ?? "";
            return (
              <article key={role.id} className="settings-role">
                <div className="settings-role-head">
                  <strong>{role.name}</strong>
                  <span className="muted">
                    {role.kind === "core" ? "Core role" : "Custom role"} ·{" "}
                    {role.executionClass.replaceAll("_", " ")}
                  </span>
                </div>
                <div className="settings-role-agents">
                  {role.assignments.length === 0 ? (
                    <span className="muted">No agent assigned</span>
                  ) : (
                    role.assignments.map((assignment) => (
                      <span key={assignment.agent} className="settings-role-chip">
                        {assignment.preferred ? <span aria-label="preferred">★</span> : null}
                        {assignment.agent}
                        <button
                          className="settings-role-chip-remove"
                          aria-label={`Remove ${assignment.agent} from ${role.name}`}
                          onClick={() =>
                            runRoleOperation(
                              () => removeRoleAssignment(role.id, assignment.agent),
                              `Removed ${assignment.agent} from ${role.name}.`
                            )
                          }
                        >
                          ×
                        </button>
                      </span>
                    ))
                  )}
                </div>
                <div className="settings-role-actions">
                  {candidates.length > 0 ? (
                    <>
                      <select
                        className="net-select"
                        aria-label={`Add agent to ${role.name}`}
                        value={selection}
                        onChange={(event) =>
                          setRoleSelection({ ...roleSelection, [role.id]: event.target.value })
                        }
                      >
                        <option value="">Add agent</option>
                        {candidates.map((name) => (
                          <option key={name} value={name}>
                            {name}
                          </option>
                        ))}
                      </select>
                      <button
                        className="button"
                        aria-label={`Assign agent to ${role.name}`}
                        disabled={!selection}
                        onClick={() => {
                          if (!selection) return;
                          const agent = selection;
                          setRoleSelection({ ...roleSelection, [role.id]: "" });
                          runRoleOperation(
                            () => addRoleAssignment(role.id, agent),
                            `Assigned ${agent} to ${role.name}.`
                          );
                        }}
                      >
                        Assign
                      </button>
                    </>
                  ) : (
                    <span className="muted">Every configured agent is assigned.</span>
                  )}
                  {role.assignments.length > 1 && (
                    <span className="settings-role-preferred">
                      {role.assignments.map((assignment) => (
                        <label key={assignment.agent}>
                          <input
                            type="radio"
                            name={`preferred-${role.id}`}
                            checked={assignment.preferred}
                            onChange={() =>
                              runRoleOperation(
                                () => setPreferredAssignment(role.id, assignment.agent),
                                `${assignment.agent} is the preferred ${role.name}.`
                              )
                            }
                          />
                          {assignment.agent}
                        </label>
                      ))}
                    </span>
                  )}
                  {role.kind === "custom" && (
                    <span className="settings-actions">
                      <button
                        className="button"
                        aria-label={`Edit ${role.name} role`}
                        onClick={() => setEditingRole(role)}
                      >
                        Edit
                      </button>
                      <button
                        className="button"
                        onClick={() => {
                          if (!window.confirm(`Delete the ${role.name} custom role?`)) return;
                          runRoleOperation(
                            () => deleteRole(role.id),
                            `Deleted the ${role.name} role.`
                          );
                        }}
                      >
                        Delete
                      </button>
                    </span>
                  )}
                </div>
              </article>
            );
          })}
        </div>
      </section>

      {editingRole && (
        <section className="section settings-form">
          <h3 className="section-title">Edit {editingRole.name}</h3>
          <RoleForm
            mode="edit"
            agents={agentNames}
            initial={editingRole}
            error={error}
            submitLabel="Save role"
            onSubmit={(value) => {
              setError(null);
              updateRole(editingRole.id, {
                name: value.name,
                description: value.description,
                executionClass: value.executionClass,
                instructions: value.instructions,
              })
                .then(() => {
                  setEditingRole(null);
                  setSaved(`Saved the ${editingRole.name} role.`);
                  reload();
                })
                .catch((err: Error) => setError(err.message));
            }}
            onCancel={() => setEditingRole(null)}
          />
        </section>
      )}

      <section className="section">
        <div className="settings-section-head">
          <h3 className="section-title">Agents</h3>
          <button className="button" onClick={() => setDraft(emptyDraft())}>
            Add agent
          </button>
        </div>
        <p className="settings-note">
          Coding CLIs that you install and authenticate yourself. The factory only runs their
          commands; it never manages model providers or API keys.
        </p>
        {agentNames.length === 0 && !draft ? (
          <div className="empty">
            <p className="empty-title">No agents configured</p>
            <p className="empty-body">Add one to start planning runs.</p>
          </div>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Command</th>
                <th>Status</th>
                <th className="settings-actions-col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {agentNames.map((name) => {
                const entry = config.agents[name];
                const status = available[name];
                return (
                  <tr key={name}>
                    <td>
                      <code>{name}</code>
                    </td>
                    <td className="run-objective">
                      <code>{[entry.command, ...entry.args].join(" ")}</code>
                    </td>
                    <td>
                      {status === undefined ? (
                        <span className="muted">checking…</span>
                      ) : (
                        <span className="agent-capability-status">
                          <span
                            className={
                              status.available
                                ? "net-ok"
                                : status.status === "broken"
                                  ? "net-warn"
                                  : "net-bad"
                            }
                          >
                            {status.available
                              ? "Installed"
                              : status.status === "broken"
                                ? "Broken installation"
                                : "Missing"}
                          </span>
                          <small>
                            Workflow {status.workflowAvailable ? "available" : "unavailable"}
                          </small>
                          <small>
                            Interactive {status.interactiveAvailable ? "available" : "unavailable"}
                          </small>
                          <details className="agent-resolution-details">
                            <summary>Resolution details</summary>
                            <small>Status: {agentResolutionStatusLabel(status.status)}</small>
                            {status.resolvedExecutable && (
                              <>
                                <span>Executable resolved</span>
                                <code>{status.resolvedExecutable}</code>
                              </>
                            )}
                            {!status.resolvedExecutable && status.resolutionError && (
                              <span className="error">{status.resolutionError}</span>
                            )}
                            {!status.resolvedExecutable && !status.resolutionError && (
                              <span>{entry.command} was not found in Factory&apos;s PATH.</span>
                            )}
                            {status.resolutionShim && (
                              <small>
                                Shim: <code>{status.resolutionShim}</code>
                              </small>
                            )}
                            {status.resolutionTarget && (
                              <small>
                                Target: <code>{status.resolutionTarget}</code>
                              </small>
                            )}
                            {status.resolutionKind && (
                              <small>
                                Resolver: <code>{status.resolutionKind}</code>
                              </small>
                            )}
                            <small>
                              Command: <code>{entry.command}</code>
                            </small>
                            <small>
                              Factory process PATH ({status.pathEntriesChecked ?? 0} entries
                              checked)
                            </small>
                          </details>
                        </span>
                      )}
                    </td>
                    <td>
                      <span className="settings-actions">
                        <button className="button" onClick={() => setDraft(draftFrom(name, entry))}>
                          Edit
                        </button>
                        <button className="button" onClick={() => removeAgent(name)}>
                          Remove
                        </button>
                      </span>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
        {agentNames.length > 0 && (
          <div className="settings-actions">
            <button className="button" onClick={refreshAvailability}>
              Test agent configuration
            </button>
          </div>
        )}
      </section>

      {draft && (
        <section className="section settings-form">
          <h3 className="section-title">
            {draft.originalName ? `Edit agent ${draft.originalName}` : "New agent"}
          </h3>
          <div className="settings-grid">
            <label className="settings-field">
              <span className="meta-label">Name</span>
              <input
                className="net-select"
                value={draft.name}
                onChange={(event) => setDraft({ ...draft, name: event.target.value })}
                placeholder="codex"
              />
            </label>
            <label className="settings-field">
              <span className="meta-label">Agent type</span>
              <select
                className="net-select"
                value={draft.kind}
                onChange={(event) => {
                  const kind = event.target.value as AgentKind;
                  if (kind === "custom") {
                    setDraft({ ...draft, kind, command: "", argsText: "", interactive: false });
                    return;
                  }
                  const preset = PRESETS[kind];
                  setDraft({
                    ...draft,
                    kind,
                    command: preset.command,
                    argsText: preset.args.join("\n"),
                    interactive: true,
                  });
                }}
              >
                {Object.entries(PRESETS).map(([kind, preset]) => (
                  <option key={kind} value={kind}>
                    {preset.label}
                  </option>
                ))}
                <option value="custom">Custom</option>
              </select>
            </label>
          </div>
          <details className="settings-advanced" open={draft.kind === "custom" || undefined}>
            <summary>{draft.kind === "custom" ? "Invocation" : "Advanced"}</summary>
            <div className="settings-grid">
              <label className="settings-field">
                <span className="meta-label">Command</span>
                <input
                  className="net-select"
                  value={draft.command}
                  onChange={(event) => setDraft({ ...draft, command: event.target.value })}
                  placeholder="codex"
                />
              </label>
              <label className="settings-field">
                <span className="meta-label">Arguments (one per line)</span>
                <textarea
                  className="net-select"
                  rows={3}
                  value={draft.argsText}
                  onChange={(event) => setDraft({ ...draft, argsText: event.target.value })}
                  placeholder={"exec"}
                />
              </label>
              {draft.kind === "custom" && (
                <>
                  <label className="settings-field">
                    <span className="meta-label">Workflow prompt transport</span>
                    <select
                      className="net-select"
                      value={draft.promptTransport}
                      onChange={(event) =>
                        setDraft({
                          ...draft,
                          promptTransport: event.target.value as PromptTransport,
                        })
                      }
                    >
                      <option value="stdin">Pass mission through stdin</option>
                      <option value="argument">Pass mission as an argument</option>
                      <option value="disabled">Interactive sessions only</option>
                    </select>
                  </label>
                  <label className="settings-field settings-checkbox">
                    <input
                      type="checkbox"
                      checked={draft.interactive}
                      onChange={(event) =>
                        setDraft({ ...draft, interactive: event.target.checked })
                      }
                    />
                    <span>Enable interactive Agent Console sessions</span>
                  </label>
                  {draft.interactive && (
                    <label className="settings-field">
                      <span className="meta-label">Interactive arguments (one per line)</span>
                      <textarea
                        className="net-select"
                        rows={2}
                        value={draft.interactiveArgsText}
                        onChange={(event) =>
                          setDraft({ ...draft, interactiveArgsText: event.target.value })
                        }
                      />
                    </label>
                  )}
                </>
              )}
              <label className="settings-field">
                <span className="meta-label">Environment (KEY=VALUE, one per line)</span>
                <textarea
                  className="net-select"
                  rows={3}
                  value={draft.envText}
                  onChange={(event) => setDraft({ ...draft, envText: event.target.value })}
                  placeholder="OPENAI_API_KEY=…"
                />
              </label>
            </div>
          </details>
          <div className="settings-actions">
            <button className="button" onClick={saveDraft}>
              {draft.originalName ? "Save changes" : "Add agent"}
            </button>
            <button className="button" onClick={() => setDraft(null)}>
              Cancel
            </button>
          </div>
        </section>
      )}
    </div>
  );
}
