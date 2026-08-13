import { useEffect, useRef, useState } from "react";
import type { AgentEntry, ConfigData } from "../types";

type AddKind = "agent" | "role" | "group" | "note";
const CORE_ROLES = ["planner", "worker", "reviewer"] as const;

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
  error,
  onClose,
  onCreateAgent,
  onCreateRole,
  onCreateVisual,
}: {
  open: boolean;
  config: ConfigData;
  error: string | null;
  onClose: () => void;
  onCreateAgent: (name: string, entry: AgentEntry) => void;
  onCreateRole: (role: string, agent: string) => void;
  onCreateVisual: (kind: "group" | "note", label: string, text: string) => void;
}) {
  const [kind, setKind] = useState<AddKind | null>(null);
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [argumentsText, setArgumentsText] = useState("");
  const [environmentText, setEnvironmentText] = useState("");
  const [role, setRole] = useState("");
  const [agent, setAgent] = useState("");
  const [text, setText] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const firstButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) window.requestAnimationFrame(() => firstButton.current?.focus());
    else {
      setKind(null);
      setValidationError(null);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [onClose, open]);

  if (!open) return null;
  const availableRoles = CORE_ROLES.filter((candidate) => !config.roles[candidate]);
  const agents = Object.keys(config.agents).sort();

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (kind === "agent") {
      const parsedEnvironment = parseEnvironment(environmentText);
      if (parsedEnvironment.error) {
        setValidationError(parsedEnvironment.error);
        return;
      }
      setValidationError(null);
      onCreateAgent(name.trim(), {
        command: command.trim(),
        args: parseLines(argumentsText),
        env: parsedEnvironment.environment,
        capabilities: [],
      });
    } else if (kind === "role") {
      onCreateRole(role, agent);
    } else if (kind) {
      onCreateVisual(kind, name.trim(), text.trim());
    }
  };

  return (
    <div className="add-node-menu" role="dialog" aria-label="Add graph node">
      <div className="add-node-menu-head">
        <strong>{kind ? `New ${kind}` : "Add node"}</strong>
        <button className="inspector-close" onClick={onClose} aria-label="Close add node menu">
          x
        </button>
      </div>
      {kind === null ? (
        <div className="add-node-types">
          {(["agent", "role", "group", "note"] as AddKind[]).map((option, index) => (
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
                  : option === "role"
                    ? "Core Factory assignment"
                    : option === "group"
                      ? "Visual organization"
                      : "Workspace context"}
              </span>
            </button>
          ))}
        </div>
      ) : (
        <form className="add-node-form" onSubmit={submit}>
          {kind === "agent" && (
            <>
              <label>
                <span>Name</span>
                <input value={name} onChange={(event) => setName(event.target.value)} required />
              </label>
              <label>
                <span>Command</span>
                <input
                  value={command}
                  onChange={(event) => setCommand(event.target.value)}
                  required
                />
              </label>
              <label>
                <span>Arguments</span>
                <textarea
                  rows={3}
                  value={argumentsText}
                  onChange={(event) => setArgumentsText(event.target.value)}
                  placeholder="One argument per line"
                />
              </label>
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
            </>
          )}
          {kind === "role" && (
            <>
              {availableRoles.length === 0 ? (
                <p className="inline-note">All supported core roles are already configured.</p>
              ) : (
                <>
                  <label>
                    <span>Role</span>
                    <select value={role} onChange={(event) => setRole(event.target.value)} required>
                      <option value="">Select a core role</option>
                      {availableRoles.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label>
                    <span>Agent</span>
                    <select
                      value={agent}
                      onChange={(event) => setAgent(event.target.value)}
                      required
                    >
                      <option value="">Select an agent</option>
                      {agents.map((option) => (
                        <option key={option} value={option}>
                          {option}
                        </option>
                      ))}
                    </select>
                  </label>
                </>
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
          {(validationError ?? error) && <p className="inline-error">{validationError ?? error}</p>}
          <div className="add-node-actions">
            <button
              className="button"
              type="submit"
              disabled={kind === "role" && availableRoles.length === 0}
            >
              Create
            </button>
            <button className="button" type="button" onClick={() => setKind(null)}>
              Back
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
