import type { FileRef } from "../files/types";
export interface ToolProfile {
  id: string;
  name: string;
  kind: "lsp" | "dap" | "shell";
  transport?: "stdio" | "tcp";
  executable: string;
  arguments: string[];
  languages: string[];
  file_patterns?: string[];
  initialization_options?: unknown;
  settings?: unknown;
  timeout_ms: number;
  enabled: boolean;
  fingerprint?: string;
}
export interface ServerInfo {
  id: string;
  root_id: string;
  profile: string;
  client_id: string;
  capabilities: Record<string, unknown>;
  alive: boolean;
  pid: number;
  documents?: Array<{ file: FileRef; version: number; content_sha256: string }>;
}
export interface TerminalInfo {
  id: string;
  root_id: string;
  profile_id: string;
  name: string;
  pid: number;
  exit_code: number | null;
}
export interface DebugInfo {
  id: string;
  root_id: string;
  profile_id: string;
  configuration_id: string;
  capabilities: Record<string, unknown>;
  alive: boolean;
  pid: number;
}
export interface LaunchConfiguration {
  id: string;
  name: string;
  profile_id: string;
  root_id: string;
  program: FileRef;
  arguments: Record<string, unknown>;
}
export interface ExtensionManifest {
  id: string;
  name: string;
  version: string;
  publisher: string;
  description: string;
  entrypoint: "contributions";
  extension_api_version: number;
  capabilities: string[];
  permissions: string[];
  supported_platforms: string[];
  dependencies?: Record<string, string>;
  contributions: {
    formatters?: Array<{ id: string; title: string; operation: string; languages: string[] }>;
    diagnostics?: Array<{
      id: string;
      needle: string;
      message: string;
      severity: "error" | "warning" | "info";
    }>;
    commands?: Array<{
      id: string;
      title: string;
      action: string;
      target: string;
      keybinding?: string;
      context_menu?: boolean;
    }>;
    languages?: Array<{ extension: string; syntax: string }>;
    views?: Array<{ id: string; title: string; text: string }>;
    settings?: Record<string, string>;
    lsp_profiles?: string[];
    dap_profiles?: string[];
  };
}
export interface InstalledExtension {
  preferences?: Record<string, string>;
  manifest: ExtensionManifest;
  sha256: string;
  provenance: string;
  installed_at: string;
  enabled: boolean;
  status: string;
  error: string | null;
}
export interface IdeState {
  profiles: ToolProfile[];
  servers: ServerInfo[];
  debug_sessions: DebugInfo[];
  terminals: TerminalInfo[];
  configurations: LaunchConfiguration[];
  extensions: InstalledExtension[];
}
export interface EditPlan {
  file: FileRef;
  expected_revision: string;
  document_version: number | null;
  before: string;
  after: string;
}
export interface RpcEvent {
  sequence: number;
  message: Record<string, unknown>;
}
export interface EventPage {
  events: RpcEvent[];
  cursor: number;
  alive: boolean;
  gap: boolean;
}
