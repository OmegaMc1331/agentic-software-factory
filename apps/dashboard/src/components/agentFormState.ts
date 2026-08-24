import type { AgentKind, ConfigData, PromptTransport } from "../types";

export interface AgentDraft {
  originalName: string | null;
  name: string;
  kind: AgentKind;
  command: string;
  args: string[];
  envText: string;
  promptTransport: PromptTransport;
  interactive: boolean;
  interactiveArgsText: string;
  maxConcurrency: string;
  capabilities: string[];
}

export function emptyAgentDraft(): AgentDraft {
  return {
    originalName: null,
    name: "",
    kind: "codex",
    command: "codex",
    args: ["exec"],
    envText: "",
    promptTransport: "stdin",
    interactive: true,
    interactiveArgsText: "",
    maxConcurrency: "",
    capabilities: [],
  };
}

export function agentArgs(entry: ConfigData["agents"][string]): string[] {
  return entry.args ?? [];
}

function inferredKind(entry: ConfigData["agents"][string]): AgentKind {
  if (entry.kind) return entry.kind;
  const args = agentArgs(entry);
  if (entry.command === "codex" && args[0] === "exec") return "codex";
  if (entry.command === "opencode" && args[0] === "run") return "open_code";
  if (entry.command === "claude" && args.some((arg) => arg === "-p" || arg === "--print"))
    return "claude_code";
  if (entry.command === "gemini" && args.some((arg) => arg === "-p" || arg === "--prompt"))
    return "gemini_cli";
  if (entry.command === "qwen" && args.some((arg) => arg === "-p" || arg === "--prompt"))
    return "qwen_code";
  return "custom";
}

export function agentDraftFrom(name: string, entry: ConfigData["agents"][string]): AgentDraft {
  return {
    originalName: name,
    name,
    kind: inferredKind(entry),
    command: entry.command,
    args: [...agentArgs(entry)],
    envText: Object.entries(entry.env ?? {})
      .map(([key, value]) => `${key}=${value}`)
      .join("\n"),
    promptTransport: entry.prompt_transport ?? "stdin",
    interactive: entry.interactive_args !== undefined || inferredKind(entry) !== "custom",
    interactiveArgsText: (entry.interactive_args ?? []).join("\n"),
    maxConcurrency: entry.max_concurrency === undefined ? "" : String(entry.max_concurrency),
    capabilities: [...(entry.capabilities ?? [])],
  };
}
