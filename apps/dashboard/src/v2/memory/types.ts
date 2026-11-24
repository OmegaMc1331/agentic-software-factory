export const categories = {
  architecture: "Architecture",
  technical_decisions: "Technical decisions",
  coding_conventions: "Coding conventions",
  project_constraints: "Project constraints",
  dependencies: "Dependencies and infrastructure",
  known_issues: "Known issues",
  previous_failures: "Previous failures and lessons",
  project_preferences: "Project preferences",
  general_facts: "General project facts",
} as const;
export type Category = keyof typeof categories;
export const statuses = ["active", "disabled", "stale", "disputed", "superseded"] as const;
export type MemoryStatus = (typeof statuses)[number];
export type SourceRef =
  | { kind: "manual"; note: string }
  | { kind: "repository_file"; rootId: string; path: string; revision: string | null }
  | { kind: "mission"; runId: number }
  | { kind: "conversation_message"; conversationId: string; messageId: number }
  | { kind: "task_attempt" | "review" | "test_evidence"; attemptId: number }
  | { kind: "agent_session"; sessionId: number }
  | { kind: "integration_commit"; runId: number; commitSha: string };
export interface Source {
  projectId: string;
  reference: SourceRef;
}
export interface Provenance {
  source: Source;
  sourceRevision: string;
  fingerprint: string;
  validationStatus: string;
  checkedAt: string;
  evidenceExcerpt?: string | null;
}
export interface MemoryInput {
  category: Category;
  title: string;
  content: string;
  subject: string;
  scope: string;
  status: MemoryStatus;
  sources: Source[];
}
export interface MemoryRecord extends Omit<MemoryInput, "sources"> {
  schemaVersion: number;
  id: string;
  projectId: string;
  revision: number;
  origin: string;
  provenance: Provenance[];
  createdAt: string;
  updatedAt: string;
  changeReason: string;
  relatedIds: string[];
}
export interface MemoryPage {
  items: MemoryRecord[];
  nextCursor: number | null;
  searchMode: string;
}
export interface Candidate {
  id: string;
  projectId: string;
  extractionId: string;
  memory: MemoryRecord;
  state: string;
  issue: string;
  acceptedMemoryId: string | null;
  createdAt: string;
}
export interface Settings {
  revision: number;
  enabled: boolean;
  extractionMode: "disabled" | "review" | "automatic";
  categories: Category[];
  retrievalEnabled: boolean;
  maxContextRecords: number;
  maxContextBytes: number;
}
export interface Usage {
  id: number;
  projectId: string;
  agentSessionId: number;
  role: string;
  memories: Array<{ memoryId: string; revision: number; contentHash: string }>;
  usedAt: string;
  contextHash: string;
}
export function memoryInput(record: MemoryRecord): MemoryInput {
  return {
    category: record.category,
    title: record.title,
    content: record.content,
    subject: record.subject,
    scope: record.scope,
    status: record.status,
    sources: record.provenance.map((p) => p.source),
  };
}
export function sourceLabel(source: Source): string {
  const r = source.reference;
  switch (r.kind) {
    case "repository_file":
      return r.path;
    case "manual":
      return r.note || "User-authored assertion";
    case "mission":
      return `Mission #${r.runId}`;
    case "conversation_message":
      return `Conversation ${r.conversationId}, message #${r.messageId}`;
    case "agent_session":
      return `AgentSession #${r.sessionId}`;
    case "integration_commit":
      return `Mission #${r.runId}, integration ${r.commitSha}`;
    default:
      return `${r.kind.replaceAll("_", " ")} #${r.attemptId}`;
  }
}
