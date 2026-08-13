import { useEffect, useRef, useState } from "react";
import type { AgentEntry, AgentKind, ConfigData, PromptTransport } from "../types";

type AddKind = "workflow" | "agent" | "role" | "group" | "note";
const CORE_ROLES = ["planner", "worker", "reviewer"] as const;
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
  error,
  initialKind,
  onClose,
  onCreateWorkflow,
  onCreateAgent,
  onCreateRole,
  onCreateVisual,
}: {
  open: boolean;
  config: ConfigData;
  error: string | null;
  initialKind?: AddKind | null;
  onClose: () => void;
  onCreateWorkflow: (objective: string) => void;
  onCreateAgent: (name: string, entry: AgentEntry) => void;
  onCreateRole: (role: string, agent: string) => void;
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
  const [role, setRole] = useState("");
  const [agent, setAgent] = useState("");
  const [text, setText] = useState("");
  const [objective, setObjective] = useState("");
  const [validationError, setValidationError] = useState<string | null>(null);
  const firstButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      setKind(initialKind ?? null);
      window.requestAnimationFrame(() => firstButton.current?.focus());
    } else {
      setKind(null);
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

  if (!open) return null;
  const availableRoles = CORE_ROLES.filter((candidate) => !config.roles[candidate]);
  const agents = Object.keys(config.agents).sort();
  const planner = config.roles.planner?.agent ?? null;

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    if (kind === "workflow") {
      if (!objective.trim()) {
        setValidationError("Describe what the Factory should build.");
        return;
      }
      setValidationError(null);
      onCreateWorkflow(objective.trim());
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
                    ? "Plan work with the configured Planner"
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
          {kind === "workflow" && (
            <>
              {planner ? (
                <>
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
                  <div className="workflow-planner-field">
                    <span>Planner</span>
                    <strong>{planner}</strong>
                    <small>From the Planner role in Factory configuration</small>
                  </div>
                </>
              ) : (
                <div className="workflow-missing-role">
                  <strong>No planner configured.</strong>
                  <p>Assign an agent to the Planner role before creating a workflow.</p>
                  <button
                    className="button"
                    type="button"
                    onClick={() => setKind(agents.length > 0 ? "role" : "agent")}
                  >
                    Configure agents
                  </button>
                </div>
              )}
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
              disabled={
                (kind === "role" && availableRoles.length === 0) ||
                (kind === "workflow" && !planner)
              }
            >
              {kind === "workflow" ? "Plan" : "Create"}
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
