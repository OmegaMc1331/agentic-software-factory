import { useCallback, useEffect, useRef, useState } from "react";
import {
  addRoleAssignment,
  createMcpServer,
  createRecipe,
  deleteRecipe,
  deleteMcpServer,
  deleteRole,
  fetchAgents,
  fetchConfig,
  fetchMcpServers,
  fetchMcpTools,
  fetchRecipes,
  fetchRoles,
  isConfigConflictError,
  mcpDiscoveryRequestTimeoutMs,
  removeRoleAssignment,
  saveConfig,
  setPreferredAssignment,
  testMcpServer,
  updateMcpServer,
  updateRecipe,
  updateRole,
} from "../api";
import {
  agentResolutionStatusLabel,
  type AgentStatusInfo,
  type ConfigData,
  type McpServerConfig,
  type McpServerView,
  type McpTool,
  type Recipe,
  type RoleInfo,
  type RoutingModeValue,
} from "../types";
import { RECIPE_ROUTING_LABELS } from "../types";
import { AgentForm } from "./AgentForm";
import { agentArgs, agentDraftFrom, emptyAgentDraft, type AgentDraft } from "./agentFormState";
import { RecipeForm } from "./RecipeForm";
import { RoleForm, type RoleFormInitial } from "./RoleForm";

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

interface McpDraft {
  originalId: string | null;
  id: string;
  transport: "stdio" | "http";
  command: string;
  argsText: string;
  url: string;
  cwd: string;
  envText: string;
  headersText: string;
  classificationsText: string;
  timeoutMs: string;
  maxResponseBytes: string;
  maxConcurrency: string;
}

function emptyMcpDraft(): McpDraft {
  return {
    originalId: null,
    id: "",
    transport: "stdio",
    command: "",
    argsText: "",
    url: "",
    cwd: "",
    envText: "",
    headersText: "",
    classificationsText: "",
    timeoutMs: "60000",
    maxResponseBytes: "262144",
    maxConcurrency: "4",
  };
}

function mcpDraftFrom(server: McpServerView): McpDraft {
  return {
    originalId: server.id,
    id: server.id,
    transport: server.transport,
    command: server.command ?? "",
    argsText: server.args.join("\n"),
    url: server.url ?? "",
    cwd: server.cwd ?? "",
    envText: Object.entries(server.env)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    headersText: Object.entries(server.headers)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    classificationsText: Object.entries(server.classifications)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    timeoutMs: String(server.timeoutMs),
    maxResponseBytes: String(server.maxResponseBytes),
    maxConcurrency: String(server.maxConcurrency),
  };
}

function mcpConfigFrom(draft: McpDraft): McpServerConfig {
  const classifications = parseEnv(draft.classificationsText) as McpServerConfig["classifications"];
  return {
    transport: draft.transport,
    ...(draft.transport === "stdio"
      ? {
          command: draft.command.trim(),
          args: parseLines(draft.argsText),
          ...(draft.cwd.trim() ? { cwd: draft.cwd.trim() } : {}),
          env: parseEnv(draft.envText),
        }
      : {
          url: draft.url.trim(),
          headers: parseEnv(draft.headersText),
        }),
    timeout_ms: Number(draft.timeoutMs),
    discovery_timeout_ms: 10000,
    max_response_bytes: Number(draft.maxResponseBytes),
    max_concurrency: Number(draft.maxConcurrency),
    classifications,
  };
}

export function SettingsView() {
  const [config, setConfig] = useState<ConfigData | null>(null);
  const [available, setAvailable] = useState<Record<string, AgentStatusInfo>>({});
  const [roles, setRoles] = useState<RoleInfo[]>([]);
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [mcpServers, setMcpServers] = useState<McpServerView[]>([]);
  const [mcpDraft, setMcpDraft] = useState<McpDraft | null>(null);
  const [mcpTools, setMcpTools] = useState<Record<string, McpTool[]>>({});
  const [mcpBusy, setMcpBusy] = useState<{
    id: string;
    action: "save" | "test" | "inspect" | "delete";
  } | null>(null);
  const mcpActionPending = useRef(false);
  const [draft, setDraft] = useState<AgentDraft | null>(null);
  const [editingRole, setEditingRole] = useState<RoleInfo | null>(null);
  const [roleSelection, setRoleSelection] = useState<Record<string, string>>({});
  const [recipeEditor, setRecipeEditor] = useState<{
    mode: "create" | "edit";
    id: string | null;
    duplicateOf: Recipe | null;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [conflict, setConflict] = useState<string | null>(null);

  const reload = useCallback(() => {
    setError(null);
    setSaved(null);
    setConflict(null);
    Promise.all([fetchConfig(), fetchAgents(), fetchRoles(), fetchRecipes(), fetchMcpServers()])
      .then(([nextConfig, list, nextRoles, nextRecipes, nextMcpServers]) => {
        setConfig(nextConfig);
        setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent])));
        setRoles(nextRoles);
        setRecipes(nextRecipes);
        setMcpServers(nextMcpServers);
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
    async (next: ConfigData, note: string): Promise<boolean> => {
      setError(null);
      setSaved(null);
      try {
        await saveConfig(next);
        setConflict(null);
        setSaved(note);
        reload();
        return true;
      } catch (reason) {
        if (isConfigConflictError(reason)) {
          // The configuration changed elsewhere after this page loaded. The
          // server kept its newer version; the draft stays open for review.
          setConflict(
            "The configuration was modified elsewhere (another tab or window) after this " +
              "page loaded. Your changes were not saved. Compare them with the latest " +
              "configuration below, then reload and reapply what still applies."
          );
        } else {
          setError((reason as Error).message);
        }
        return false;
      }
    },
    [reload]
  );

  const saveAgentDraft = useCallback(
    async (name: string, entry: ConfigData["agents"][string], originalName: string | null) => {
      if (!config) return false;
      if (originalName && !config.agents[originalName]) {
        setError(`Agent '${originalName}' no longer exists.`);
        return false;
      }
      if (name !== originalName && config.agents[name]) {
        setError(`Agent '${name}' already exists.`);
        return false;
      }
      const agents = { ...config.agents };
      if (originalName && originalName !== name) {
        delete agents[originalName];
      }
      agents[name] = entry;
      const role_assignments =
        originalName && originalName !== name
          ? config.role_assignments.map((assignment) =>
              assignment.agent === originalName ? { ...assignment, agent: name } : assignment
            )
          : config.role_assignments;
      const saved = await apply(
        { ...config, agents, role_assignments },
        originalName ? `Saved agent ${name}.` : `Added agent ${name}.`
      );
      if (saved) setDraft(null);
      return saved;
    },
    [apply, config]
  );

  const removeAgent = (name: string) => {
    if (!config) return;
    const agents = { ...config.agents };
    delete agents[name];
    const role_assignments = config.role_assignments.filter((entry) => entry.agent !== name);
    setDraft(null);
    void apply({ ...config, agents, role_assignments }, `Removed agent ${name}.`);
  };

  const refreshAvailability = () => {
    fetchAgents()
      .then((list) => setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent]))))
      .catch((err: Error) => setError(err.message));
  };

  const saveMcpDraft = async () => {
    if (!mcpDraft || mcpActionPending.current) return;
    const id = mcpDraft.id.trim();
    if (!id) {
      setError("MCP server id is required.");
      return;
    }
    if (mcpDraft.transport === "stdio" && !mcpDraft.command.trim()) {
      setError("A structured command is required for a stdio MCP server.");
      return;
    }
    if (mcpDraft.transport === "http" && !mcpDraft.url.trim()) {
      setError("A URL is required for an HTTP MCP server.");
      return;
    }
    mcpActionPending.current = true;
    setMcpBusy({ id, action: "save" });
    setError(null);
    try {
      if (mcpDraft.originalId) {
        await updateMcpServer(mcpDraft.originalId, mcpConfigFrom(mcpDraft));
      } else {
        await createMcpServer(id, mcpConfigFrom(mcpDraft));
      }
      setMcpDraft(null);
      setSaved(`${mcpDraft.originalId ? "Saved" : "Added"} MCP server ${id}.`);
      reload();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      mcpActionPending.current = false;
      setMcpBusy(null);
    }
  };

  const testMcp = async (id: string) => {
    if (mcpActionPending.current) return;
    const server = mcpServers.find((candidate) => candidate.id === id);
    mcpActionPending.current = true;
    setMcpBusy({ id, action: "test" });
    setError(null);
    try {
      const tested = await testMcpServer(id, {
        timeoutMs: mcpDiscoveryRequestTimeoutMs(server?.discoveryTimeoutMs),
      });
      setMcpServers((current) => current.map((entry) => (entry.id === id ? tested : entry)));
      if (tested.status === "connected") setSaved(`${id} connected; ${tested.toolCount} tools.`);
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      mcpActionPending.current = false;
      setMcpBusy(null);
    }
  };

  const inspectMcp = async (id: string) => {
    if (mcpActionPending.current) return;
    const server = mcpServers.find((candidate) => candidate.id === id);
    mcpActionPending.current = true;
    setMcpBusy({ id, action: "inspect" });
    setError(null);
    try {
      const tools = await fetchMcpTools(id, {
        timeoutMs: mcpDiscoveryRequestTimeoutMs(server?.discoveryTimeoutMs),
      });
      setMcpTools((current) => ({ ...current, [id]: tools }));
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      mcpActionPending.current = false;
      setMcpBusy(null);
    }
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
      {conflict && (
        <div className="empty" role="alert" data-testid="config-conflict">
          <p className="empty-title">Configuration changed elsewhere</p>
          <p className="error">{conflict}</p>
          <div className="empty-actions">
            <button
              className="button"
              onClick={() => {
                setConflict(null);
                reload();
              }}
            >
              Reload latest configuration
            </button>
            <button className="button" onClick={() => setConflict(null)}>
              Keep my changes open
            </button>
          </div>
        </div>
      )}

      <section className="section">
        <h3 className="section-title">Routing</h3>
        <p className="settings-note">
          How the scheduler picks the agent for each dispatch. Candidate eligibility (role, policy,
          availability, capacity) always comes before performance; unreliable history falls back to
          capacity-aware round-robin.
        </p>
        {config?.routing && (
          <div className="settings-routing">
            <label htmlFor="routing-mode">Mode</label>
            <select
              id="routing-mode"
              value={config.routing.mode}
              onChange={(event) =>
                config?.routing &&
                apply(
                  {
                    ...config,
                    routing: {
                      mode: event.target.value as RoutingModeValue,
                      exploration: config.routing.exploration,
                    },
                  },
                  "Routing mode saved."
                )
              }
            >
              <option value="round_robin">Round-robin (deterministic)</option>
              <option value="performance">Performance-aware</option>
              <option value="manual">Manual (preferred / pinned)</option>
            </select>
            <label htmlFor="routing-exploration">Exploration</label>
            <select
              id="routing-exploration"
              value={config.routing.exploration ? "enabled" : "disabled"}
              onChange={(event) =>
                config?.routing &&
                apply(
                  {
                    ...config,
                    routing: {
                      mode: config.routing.mode,
                      exploration: event.target.value === "enabled",
                    },
                  },
                  "Routing exploration saved."
                )
              }
            >
              <option value="enabled">Enabled</option>
              <option value="disabled">Disabled</option>
            </select>
            <p className="settings-note">
              Fallback: capacity-aware round robin. Minimum reliable samples: evaluation defaults
              (10). Performance metrics come from the local Performance view.
            </p>
          </div>
        )}
      </section>

      <section className="section" aria-label="External Tools MCP">
        <div className="settings-section-head">
          <h3 className="section-title">External Tools / MCP</h3>
          {!mcpDraft && (
            <button className="button" onClick={() => setMcpDraft(emptyMcpDraft())}>
              Add server
            </button>
          )}
        </div>
        <p className="settings-note">
          Factory discovers and invokes external tools through scoped role policy. Commands use
          structured arguments; secrets must be environment references such as{" "}
          <code>{"${GITHUB_TOKEN}"}</code> and are never shown here as resolved values.
        </p>
        {mcpDraft && (
          <form
            className="recipe-form settings-form"
            onSubmit={(event) => {
              event.preventDefault();
              void saveMcpDraft();
            }}
          >
            <div className="settings-routing recipe-form-row">
              <label>
                <span>Server id</span>
                <input
                  value={mcpDraft.id}
                  disabled={mcpDraft.originalId !== null}
                  onChange={(event) => setMcpDraft({ ...mcpDraft, id: event.target.value })}
                  placeholder="github"
                />
              </label>
              <label>
                <span>Transport</span>
                <select
                  value={mcpDraft.transport}
                  onChange={(event) =>
                    setMcpDraft({
                      ...mcpDraft,
                      transport: event.target.value as "stdio" | "http",
                    })
                  }
                >
                  <option value="stdio">stdio</option>
                  <option value="http">Streamable HTTP</option>
                </select>
              </label>
            </div>
            {mcpDraft.transport === "stdio" ? (
              <>
                <label>
                  <span>Command</span>
                  <input
                    value={mcpDraft.command}
                    onChange={(event) => setMcpDraft({ ...mcpDraft, command: event.target.value })}
                    placeholder="github-mcp-server"
                  />
                </label>
                <label>
                  <span>Arguments (one per line)</span>
                  <textarea
                    rows={3}
                    value={mcpDraft.argsText}
                    onChange={(event) => setMcpDraft({ ...mcpDraft, argsText: event.target.value })}
                  />
                </label>
                <label>
                  <span>Working directory (optional, existing project directory)</span>
                  <input
                    value={mcpDraft.cwd}
                    onChange={(event) => setMcpDraft({ ...mcpDraft, cwd: event.target.value })}
                  />
                </label>
                <label>
                  <span>Environment references (NAME=${"${NAME}"}, one per line)</span>
                  <textarea
                    rows={3}
                    value={mcpDraft.envText}
                    onChange={(event) => setMcpDraft({ ...mcpDraft, envText: event.target.value })}
                  />
                </label>
              </>
            ) : (
              <>
                <label>
                  <span>Streamable HTTP URL</span>
                  <input
                    type="url"
                    value={mcpDraft.url}
                    onChange={(event) => setMcpDraft({ ...mcpDraft, url: event.target.value })}
                    placeholder="http://127.0.0.1:3001/mcp"
                  />
                </label>
                <label>
                  <span>Header references (NAME=${"${NAME}"}, one per line)</span>
                  <textarea
                    rows={3}
                    value={mcpDraft.headersText}
                    onChange={(event) =>
                      setMcpDraft({ ...mcpDraft, headersText: event.target.value })
                    }
                  />
                </label>
              </>
            )}
            <div className="settings-routing recipe-form-row">
              <label>
                <span>Call timeout (ms)</span>
                <input
                  type="number"
                  min={100}
                  max={600000}
                  value={mcpDraft.timeoutMs}
                  onChange={(event) => setMcpDraft({ ...mcpDraft, timeoutMs: event.target.value })}
                />
              </label>
              <label>
                <span>Response limit (bytes)</span>
                <input
                  type="number"
                  min={1024}
                  max={1048576}
                  value={mcpDraft.maxResponseBytes}
                  onChange={(event) =>
                    setMcpDraft({ ...mcpDraft, maxResponseBytes: event.target.value })
                  }
                />
              </label>
              <label>
                <span>Per-server concurrency</span>
                <input
                  type="number"
                  min={1}
                  max={32}
                  value={mcpDraft.maxConcurrency}
                  onChange={(event) =>
                    setMcpDraft({ ...mcpDraft, maxConcurrency: event.target.value })
                  }
                />
              </label>
            </div>
            <label>
              <span>Classification overrides (tool=classification)</span>
              <textarea
                rows={3}
                value={mcpDraft.classificationsText}
                onChange={(event) =>
                  setMcpDraft({ ...mcpDraft, classificationsText: event.target.value })
                }
                placeholder={"create_issue=write\ndelete_issue=destructive"}
              />
            </label>
            <div className="settings-actions">
              <button className="button" type="submit" disabled={mcpBusy !== null}>
                {mcpBusy?.action === "save" ? "Saving…" : "Save server"}
              </button>
              <button
                className="button"
                type="button"
                onClick={() => setMcpDraft(null)}
                disabled={mcpBusy !== null}
              >
                Cancel
              </button>
            </div>
          </form>
        )}
        <div className="settings-roles">
          {mcpServers.length === 0 && !mcpDraft ? (
            <p className="muted">No MCP servers configured.</p>
          ) : (
            mcpServers.map((server) => (
              <article key={server.id} className="settings-role">
                <div className="settings-role-head">
                  <strong>{server.id}</strong>
                  <span
                    className={
                      server.status === "connected"
                        ? "net-ok"
                        : server.status === "unavailable"
                          ? "net-bad"
                          : "muted"
                    }
                  >
                    {server.status === "connected"
                      ? `Connected Â· ${server.toolCount} tools`
                      : server.status === "unavailable"
                        ? "Unavailable"
                        : "Not tested"}
                  </span>
                </div>
                <p className="settings-note">
                  {server.transport === "stdio"
                    ? [server.command, ...server.args].filter(Boolean).join(" ")
                    : server.url}
                </p>
                {server.lastError && <p className="inline-error">{server.lastError}</p>}
                <div className="settings-role-actions">
                  <span className="settings-actions">
                    <button
                      className="button"
                      disabled={mcpBusy !== null}
                      aria-busy={mcpBusy?.id === server.id && mcpBusy.action === "test"}
                      onClick={() => void testMcp(server.id)}
                    >
                      {mcpBusy?.id === server.id && mcpBusy.action === "test"
                        ? "Testing…"
                        : "Test connection"}
                    </button>
                    <button
                      className="button"
                      disabled={mcpBusy !== null}
                      aria-busy={mcpBusy?.id === server.id && mcpBusy.action === "inspect"}
                      onClick={() => void inspectMcp(server.id)}
                    >
                      {mcpBusy?.id === server.id && mcpBusy.action === "inspect"
                        ? "Inspecting…"
                        : "Inspect tools"}
                    </button>
                    <button className="button" onClick={() => setMcpDraft(mcpDraftFrom(server))}>
                      Edit
                    </button>
                    <button
                      className="button"
                      onClick={() => {
                        if (mcpActionPending.current) return;
                        if (!window.confirm(`Remove the ${server.id} MCP server?`)) return;
                        mcpActionPending.current = true;
                        setMcpBusy({ id: server.id, action: "delete" });
                        deleteMcpServer(server.id)
                          .then(() => {
                            mcpActionPending.current = false;
                            setMcpBusy(null);
                            reload();
                          })
                          .catch((reason: Error) => {
                            mcpActionPending.current = false;
                            setMcpBusy(null);
                            setError(reason.message);
                          });
                      }}
                    >
                      Remove
                    </button>
                  </span>
                </div>
                {mcpTools[server.id] && (
                  <div className="role-agent-list" aria-label={`${server.id} tools`}>
                    {mcpTools[server.id].map((tool) => (
                      <details key={tool.qualifiedName} className="agent-resolution-details">
                        <summary>
                          <code>{tool.qualifiedName}</code> · {tool.classification}
                        </summary>
                        <p className="settings-note">{tool.description || "No description."}</p>
                        <pre>{JSON.stringify(tool.inputSchema, null, 2)}</pre>
                      </details>
                    ))}
                  </div>
                )}
              </article>
            ))
          )}
        </div>
      </section>

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
            initial={{
              ...editingRole,
              policyPreset: (editingRole.policyPreset as RoleFormInitial["policyPreset"]) ?? null,
            }}
            error={error}
            submitLabel="Save role"
            onSubmit={(value) => {
              setError(null);
              updateRole(editingRole.id, {
                name: value.name,
                description: value.description,
                executionClass: value.executionClass,
                instructions: value.instructions,
                policyPreset: value.policyPreset ?? undefined,
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
          <h3 className="section-title">Recipes</h3>
          {!recipeEditor && (
            <button
              className="button"
              onClick={() => setRecipeEditor({ mode: "create", id: null, duplicateOf: null })}
            >
              New recipe
            </button>
          )}
        </div>
        <p className="settings-note">
          Reusable workflow configuration: roles, routing, parallelism, policy presets, and Planner
          guidance. A recipe never stores tasks — the Planner still builds each plan from the
          objective. Custom recipes live in <code>.factory/recipes.toml</code>; built-ins are
          read-only but can be duplicated.
        </p>
        {recipeEditor ? (
          <RecipeForm
            mode={recipeEditor.mode}
            roles={roles}
            initial={
              recipeEditor.duplicateOf
                ? {
                    name: `${recipeEditor.duplicateOf.name} (copy)`,
                    description: recipeEditor.duplicateOf.description,
                    roles: recipeEditor.duplicateOf.roles,
                    routing: recipeEditor.duplicateOf.routing,
                    maxParallelTasks: recipeEditor.duplicateOf.maxParallelTasks,
                    policyPresets: recipeEditor.duplicateOf.policyPresets,
                    plannerGuidance: recipeEditor.duplicateOf.plannerGuidance,
                    tools: recipeEditor.duplicateOf.tools ?? [],
                  }
                : recipeEditor.mode === "edit"
                  ? (() => {
                      const recipe = recipes.find((candidate) => candidate.id === recipeEditor.id);
                      return recipe
                        ? {
                            name: recipe.name,
                            description: recipe.description,
                            roles: recipe.roles,
                            routing: recipe.routing,
                            maxParallelTasks: recipe.maxParallelTasks,
                            policyPresets: recipe.policyPresets,
                            plannerGuidance: recipe.plannerGuidance,
                            tools: recipe.tools ?? [],
                          }
                        : undefined;
                    })()
                  : undefined
            }
            error={error}
            submitLabel={recipeEditor.mode === "edit" ? "Save recipe" : "Create recipe"}
            onSubmit={(value) => {
              setError(null);
              const done = () => {
                setRecipeEditor(null);
                setSaved(`Saved the ${value.name} recipe.`);
                reload();
              };
              const failed = (err: Error) => setError(err.message);
              if (recipeEditor.mode === "edit" && recipeEditor.id) {
                updateRecipe(recipeEditor.id, value).then(done).catch(failed);
              } else {
                createRecipe(value).then(done).catch(failed);
              }
            }}
            onCancel={() => setRecipeEditor(null)}
          />
        ) : (
          <div className="settings-roles">
            {recipes.map((recipe) => (
              <article key={recipe.id} className="settings-role">
                <div className="settings-role-head">
                  <strong>{recipe.name}</strong>
                  <span className="muted">
                    {recipe.builtin ? "Built-in · read-only" : "Custom"} ·{" "}
                    {RECIPE_ROUTING_LABELS[recipe.routing]}
                    {recipe.maxParallelTasks ? ` · parallelism ${recipe.maxParallelTasks}` : ""}
                  </span>
                </div>
                {recipe.description && <p className="settings-note">{recipe.description}</p>}
                <div className="settings-role-agents">
                  {recipe.roles.length === 0 ? (
                    <span className="muted">No pre-selected roles</span>
                  ) : (
                    recipe.roles.map((roleId) => (
                      <span key={roleId} className="settings-role-chip">
                        {roles.find((role) => role.id === roleId)?.name ?? roleId}
                      </span>
                    ))
                  )}
                </div>
                <div className="settings-role-actions">
                  <span className="settings-actions">
                    <button
                      className="button"
                      aria-label={`Duplicate ${recipe.name}`}
                      onClick={() =>
                        setRecipeEditor({
                          mode: "create",
                          id: null,
                          duplicateOf: recipe,
                        })
                      }
                    >
                      Duplicate
                    </button>
                    {!recipe.builtin && (
                      <>
                        <button
                          className="button"
                          aria-label={`Edit ${recipe.name} recipe`}
                          onClick={() =>
                            setRecipeEditor({ mode: "edit", id: recipe.id, duplicateOf: null })
                          }
                        >
                          Edit
                        </button>
                        <button
                          className="button"
                          onClick={() => {
                            if (!window.confirm(`Delete the ${recipe.name} recipe?`)) return;
                            runRoleOperation(
                              () => deleteRecipe(recipe.id),
                              `Deleted the ${recipe.name} recipe.`
                            );
                          }}
                        >
                          Delete
                        </button>
                      </>
                    )}
                  </span>
                </div>
              </article>
            ))}
          </div>
        )}
      </section>

      <section className="section">
        <div className="settings-section-head">
          <h3 className="section-title">Agents</h3>
          <button className="button" onClick={() => setDraft(emptyAgentDraft())}>
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
                      <code>{[entry.command, ...agentArgs(entry)].join(" ")}</code>
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
                        <button
                          className="button"
                          onClick={() => setDraft(agentDraftFrom(name, entry))}
                        >
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
        <AgentForm
          initial={draft}
          error={error}
          onSave={saveAgentDraft}
          onCancel={() => setDraft(null)}
        />
      )}
    </div>
  );
}
