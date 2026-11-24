export interface FileRef {
  project_id: string;
  root_id: string;
  path: string;
}
export interface RootInfo {
  id: string;
  kind: "repository" | "task_worktree" | "integration_snapshot";
  label: string;
  read_only: boolean;
}
export interface FileSnapshot {
  file: FileRef;
  revision: string;
  content: string | null;
  size: number;
  modified: string;
  encoding: string;
  bom: boolean;
  newline: string;
  read_only: boolean;
  reason: string | null;
  mode: "text" | "preview" | "metadata";
  durability: string | null;
}
export interface FileEntry {
  path: string;
  name: string;
  kind: string;
  size: number;
  hidden: boolean;
  ignored: boolean;
  protected: boolean;
  revision: string;
  git: string | null;
}
export interface DirectoryPage {
  entries: FileEntry[];
  next: string | null;
  complete: boolean;
  reason: string | null;
  git_available: boolean;
}
export interface RootCatalog {
  warning?: string | null;
  roots: RootInfo[];
  limits: {
    edit_bytes: number;
    preview_bytes: number;
    directory_entries: number;
    draft_bytes: number;
    draft_records: number;
  };
  worktree_policy: string;
}
export interface Draft {
  schema: 1;
  id: string;
  file: FileRef;
  base_revision: string;
  base_content: string;
  content: string;
  newline: string;
  bom: boolean;
  dirty: boolean;
}
export interface DraftEnvelope {
  revision: number;
  document: Draft;
}
export interface Recovery {
  id: string;
  root_id: string;
  path: string;
  kind: string;
  phase: string;
  temporary: string;
  backup: string;
  created_at: string;
}
export interface MoveResult {
  from: FileRef;
  to: FileRef;
  kind: "file" | "directory";
}
export interface SearchRequest {
  id: string;
  root_id: string;
  query: string;
  content: boolean;
  case_sensitive: boolean;
  regex: boolean;
  include_hidden: boolean;
  path_filter: string;
}
export interface SearchHit {
  file: FileRef;
  line: number | null;
  column: number | null;
  preview: string;
  rank: number;
}
export interface SearchResponse {
  hits: SearchHit[];
  complete: boolean;
  reason: string | null;
  scanned: number;
  bytes_read: number;
  elapsed_ms: number;
}
export const token = (prefix = "d") => `${prefix}_${crypto.randomUUID().replaceAll("-", "")}`;
export const validFile = (value: unknown): value is FileRef => {
  if (!value || typeof value !== "object") return false;
  const file = value as FileRef;
  return (
    /^p_[a-f0-9]{32}$/.test(file.project_id) &&
    /^r_[a-f0-9]{32}$/.test(file.root_id) &&
    typeof file.path === "string" &&
    file.path.length > 0 &&
    file.path.length <= 2048 &&
    !file.path.includes("\\") &&
    file.path.split("/").every((part) => part && part !== "." && part !== "..") &&
    !file.path.includes(":") &&
    [...file.path].every((character) => character.charCodeAt(0) >= 32)
  );
};
export const fileKey = (file: FileRef) =>
  JSON.stringify([file.project_id, file.root_id, file.path]);
export function fileTitle(path: string, rootLabel = "Repository") {
  const name = path.split("/").at(-1) ?? path;
  const compact = name.length > 100 ? `${[...name].slice(0, 70).join("")}…` : name;
  return compact + (rootLabel === "Repository" ? "" : ` · ${[...rootLabel].slice(0, 30).join("")}`);
}
export const validDraft = (value: unknown): value is Draft => {
  if (!value || typeof value !== "object") return false;
  const draft = value as Draft;
  return (
    draft.schema === 1 &&
    /^d_[a-f0-9]{32}$/.test(draft.id) &&
    validFile(draft.file) &&
    typeof draft.base_revision === "string" &&
    draft.base_revision.length <= 128 &&
    typeof draft.base_content === "string" &&
    draft.base_content.length <= 1_048_576 &&
    typeof draft.content === "string" &&
    draft.content.length <= 1_048_576 &&
    ["lf", "crlf", "mixed", "none"].includes(draft.newline) &&
    typeof draft.bom === "boolean" &&
    typeof draft.dirty === "boolean"
  );
};
