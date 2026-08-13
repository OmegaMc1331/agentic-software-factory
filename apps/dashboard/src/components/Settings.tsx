import { useCallback, useEffect, useState } from "react";
import { fetchAgents, fetchConfig, saveConfig } from "../api";
import type { ConfigData } from "../types";

const ROLES = ["planner", "worker", "reviewer"] as const;

interface Draft {
  originalName: string | null;
  name: string;
  command: string;
  argsText: string;
  envText: string;
}

function emptyDraft(): Draft {
  return { originalName: null, name: "", command: "", argsText: "", envText: "" };
}

function draftFrom(name: string, entry: ConfigData["agents"][string]): Draft {
  return {
    originalName: name,
    name,
    command: entry.command,
    argsText: entry.args.join("\n"),
    envText: Object.entries(entry.env)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
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
  const [available, setAvailable] = useState<Record<string, boolean>>({});
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const reload = useCallback(() => {
    setError(null);
    setSaved(null);
    Promise.all([fetchConfig(), fetchAgents()])
      .then(([nextConfig, list]) => {
        setConfig(nextConfig);
        setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent.available])));
      })
      .catch((err: Error) => setError(err.message));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

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
    agents[name] = {
      command: draft.command.trim(),
      args: parseLines(draft.argsText),
      env: parseEnv(draft.envText),
      capabilities: draft.originalName ? config.agents[draft.originalName]?.capabilities : [],
    };
    const editing = draft.originalName !== null;
    setDraft(null);
    apply({ ...config, agents }, editing ? `Saved agent ${name}.` : `Added agent ${name}.`);
  };

  const removeAgent = (name: string) => {
    if (!config) return;
    const agents = { ...config.agents };
    delete agents[name];
    const roles = Object.fromEntries(
      Object.entries(config.roles).filter(([, role]) => role.agent !== name)
    );
    setDraft(null);
    apply({ agents, roles }, `Removed agent ${name}.`);
  };

  const setRole = (role: string, agent: string) => {
    if (!config) return;
    const roles = { ...config.roles };
    if (agent === "") {
      delete roles[role];
    } else {
      roles[role] = { agent };
    }
    apply({ ...config, roles }, `Saved the ${role} role.`);
  };

  const refreshAvailability = () => {
    fetchAgents()
      .then((list) =>
        setAvailable(Object.fromEntries(list.map((agent) => [agent.name, agent.available])))
      )
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
          Assign a configured coding agent to each role. The config is written to{" "}
          <code>.factory/config.toml</code>.
        </p>
        <table className="table">
          <tbody>
            {ROLES.map((role) => (
              <tr key={role}>
                <td className="settings-role-name">{role}</td>
                <td>
                  <select
                    className="net-select"
                    value={config.roles[role]?.agent ?? ""}
                    onChange={(event) => setRole(role, event.target.value)}
                  >
                    <option value="">— none —</option>
                    {agentNames.map((name) => (
                      <option key={name} value={name}>
                        {name}
                      </option>
                    ))}
                  </select>
                </td>
                <td className="muted">
                  {config.roles[role] ? config.roles[role].agent : "not assigned"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

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
                      ) : status ? (
                        <span className="net-ok">available</span>
                      ) : (
                        <span className="net-bad">missing</span>
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
              Test executable availability
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
