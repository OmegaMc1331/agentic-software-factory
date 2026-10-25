import { useEffect, useId, useRef, useState } from "react";
import type { AgentEntry, AgentKind, PromptTransport } from "../types";
import { parseAgentEnv, type EnvValidationError } from "./agentEnv";
import { type AgentDraft } from "./agentFormState";

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

function parseLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

interface AgentFormProps {
  initial: AgentDraft;
  error?: string | null;
  onSave: (
    name: string,
    entry: AgentEntry,
    originalName: string | null
  ) => Promise<boolean> | boolean;
  onCancel: () => void;
}

export function AgentForm({ initial, error, onSave, onCancel }: AgentFormProps) {
  const [draft, setDraft] = useState<AgentDraft>(initial);
  const [localError, setLocalError] = useState<string | null>(null);
  const [envErrors, setEnvErrors] = useState<EnvValidationError[] | null>(null);
  const [saving, setSaving] = useState(false);
  const envErrorId = useId();
  const envFieldRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setDraft(initial);
    setLocalError(null);
    setEnvErrors(null);
  }, [initial]);

  const updateArgument = (index: number, value: string) => {
    setDraft((current) => ({
      ...current,
      args: current.args.map((argument, i) => (i === index ? value : argument)),
    }));
  };

  const removeArgument = (index: number) => {
    setDraft((current) => ({
      ...current,
      args: current.args.filter((_, i) => i !== index),
    }));
  };

  const moveArgument = (index: number, offset: -1 | 1) => {
    setDraft((current) => {
      const target = index + offset;
      if (target < 0 || target >= current.args.length) return current;
      const args = [...current.args];
      [args[index], args[target]] = [args[target], args[index]];
      return { ...current, args };
    });
  };

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = draft.name.trim();
    if (!name) {
      setLocalError("Agent name is required.");
      return;
    }
    const command = draft.command.trim();
    if (!command) {
      setLocalError("Command is required.");
      return;
    }
    const maxConcurrencyText = draft.maxConcurrency.trim();
    let maxConcurrency: number | undefined;
    if (maxConcurrencyText) {
      maxConcurrency = Number(maxConcurrencyText);
      if (!Number.isInteger(maxConcurrency) || maxConcurrency < 1 || maxConcurrency > 32) {
        setLocalError("Max concurrency must be an integer between 1 and 32.");
        return;
      }
    }
    const parsedEnv = parseAgentEnv(draft.envText);
    if (!parsedEnv.ok) {
      setLocalError(null);
      setEnvErrors(parsedEnv.errors);
      envFieldRef.current?.focus();
      return;
    }
    setLocalError(null);
    setEnvErrors(null);
    setSaving(true);
    const entry: AgentEntry = {
      kind: draft.kind,
      command,
      args: draft.args.filter((argument) => argument.trim().length > 0),
      env: parsedEnv.env,
      capabilities: draft.capabilities,
      ...(draft.kind === "custom"
        ? {
            prompt_transport: draft.promptTransport,
            ...(draft.interactive
              ? { interactive_args: parseLines(draft.interactiveArgsText) }
              : {}),
          }
        : {}),
      ...(maxConcurrency === undefined ? {} : { max_concurrency: maxConcurrency }),
    };
    try {
      await onSave(name, entry, draft.originalName);
    } catch (reason) {
      setLocalError((reason as Error).message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="section settings-form" aria-label="Agent configuration form">
      <h3 className="section-title">
        {draft.originalName ? `Edit agent ${draft.originalName}` : "New agent"}
      </h3>
      {(localError ?? error) && (
        <p className="error" role="alert">
          {localError ?? error}
        </p>
      )}
      <form onSubmit={submit}>
        <div className="settings-grid">
          <label className="settings-field">
            <span className="meta-label">Name</span>
            <input
              className="net-select"
              aria-label="Name"
              value={draft.name}
              onChange={(event) => setDraft({ ...draft, name: event.target.value })}
              placeholder="codex"
            />
          </label>
          <label className="settings-field">
            <span className="meta-label">Agent type</span>
            <select
              className="net-select"
              aria-label="Agent type"
              value={draft.kind}
              onChange={(event) => {
                const kind = event.target.value as AgentKind;
                if (kind === "custom") {
                  setDraft({ ...draft, kind, command: "", args: [], interactive: false });
                  return;
                }
                const preset = PRESETS[kind];
                setDraft({
                  ...draft,
                  kind,
                  command: preset.command,
                  args: [...preset.args],
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
                aria-label="Command"
                value={draft.command}
                onChange={(event) => setDraft({ ...draft, command: event.target.value })}
                placeholder="codex"
              />
            </label>
            <div className="settings-field">
              <span className="meta-label">Arguments</span>
              <div className="settings-argument-list">
                {draft.args.map((argument, index) => (
                  <div className="settings-argument-row" key={index}>
                    <input
                      className="net-select"
                      aria-label={`Argument ${index + 1}`}
                      value={argument}
                      onChange={(event) => updateArgument(index, event.target.value)}
                      placeholder="argv argument"
                    />
                    <span className="settings-actions">
                      <button
                        className="button"
                        type="button"
                        aria-label={`Move argument ${index + 1} up`}
                        disabled={index === 0}
                        onClick={() => moveArgument(index, -1)}
                      >
                        ↑
                      </button>
                      <button
                        className="button"
                        type="button"
                        aria-label={`Move argument ${index + 1} down`}
                        disabled={index === draft.args.length - 1}
                        onClick={() => moveArgument(index, 1)}
                      >
                        ↓
                      </button>
                      <button
                        className="button"
                        type="button"
                        aria-label={`Remove argument ${index + 1}`}
                        onClick={() => removeArgument(index)}
                      >
                        Remove
                      </button>
                    </span>
                  </div>
                ))}
                <button
                  className="button"
                  type="button"
                  onClick={() => setDraft({ ...draft, args: [...draft.args, ""] })}
                >
                  + Add argument
                </button>
              </div>
              <small className="muted">
                Each row is one argv argument. Spaces are preserved; blank rows are ignored.
              </small>
            </div>
            {draft.kind === "custom" && (
              <>
                <label className="settings-field">
                  <span className="meta-label">Workflow prompt transport</span>
                  <select
                    className="net-select"
                    aria-label="Workflow prompt transport"
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
                    onChange={(event) => setDraft({ ...draft, interactive: event.target.checked })}
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
                ref={envFieldRef}
                className="net-select"
                aria-label="Environment (KEY=VALUE, one per line)"
                aria-invalid={envErrors ? true : undefined}
                aria-describedby={envErrors ? envErrorId : undefined}
                rows={3}
                value={draft.envText}
                onChange={(event) => {
                  const envText = event.target.value;
                  setDraft((current) => ({ ...current, envText }));
                  setEnvErrors((current) => {
                    if (!current) return current;
                    const result = parseAgentEnv(envText);
                    return result.ok ? null : result.errors;
                  });
                }}
                placeholder="OPENAI_API_KEY=…"
              />
              {envErrors && (
                <div className="error settings-env-errors" id={envErrorId} role="alert">
                  <ul>
                    {envErrors.map((validationError) => (
                      <li key={`${validationError.line}-${validationError.message}`}>
                        Line {validationError.line}: {validationError.message}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <small className="muted">
                Names use letters, digits, and underscores and cannot start with a digit. Duplicate
                names are rejected, including case variants; surrounding whitespace is trimmed.
              </small>
            </label>
            <label className="settings-field">
              <span className="meta-label">Max concurrency (optional)</span>
              <input
                className="net-select"
                aria-label="Max concurrency (optional)"
                type="number"
                min={1}
                max={32}
                step={1}
                value={draft.maxConcurrency}
                onChange={(event) => setDraft({ ...draft, maxConcurrency: event.target.value })}
                placeholder="1"
              />
              <small className="muted">Automation processes allowed for this agent (1–32).</small>
            </label>
            <label className="settings-field">
              <span className="meta-label">Capabilities (one per line)</span>
              <textarea
                className="net-select"
                aria-label="Capabilities (one per line)"
                rows={2}
                value={draft.capabilities.join("\n")}
                onChange={(event) =>
                  setDraft({ ...draft, capabilities: parseLines(event.target.value) })
                }
              />
            </label>
          </div>
        </details>
        <div className="settings-actions">
          <button className="button" type="submit" disabled={saving}>
            {saving ? "Saving…" : draft.originalName ? "Save changes" : "Add agent"}
          </button>
          <button className="button" type="button" onClick={onCancel} disabled={saving}>
            Cancel
          </button>
        </div>
      </form>
    </section>
  );
}
