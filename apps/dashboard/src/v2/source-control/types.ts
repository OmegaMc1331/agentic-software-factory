import type { RootInfo } from "../files/types";
import type { TaskAttempt } from "../../types";

export interface GitChange {
  path: string;
  originalPath: string | null;
  index: string;
  working: string;
  conflict: boolean;
  supported: boolean;
}
export interface GitStatus {
  head: string | null;
  branch: string | null;
  upstream: string | null;
  ahead: number | null;
  behind: number | null;
  indexRevision: string;
  generation: string;
  changes: GitChange[];
  operation: string | null;
  observation: string;
}
export type Comparison = "working_index" | "index_head" | "working_head" | "commits";
export interface GitDiff {
  path: string;
  comparison: Comparison;
  base: string;
  target: string;
  generation: string;
  version: string;
  before: string | null;
  after: string | null;
  patch: string;
  hunks: { id: string; header: string; patch: string }[];
  limited: boolean;
  reason: string | null;
}
export interface GitBranch {
  name: string;
  commit: string;
  current: boolean;
  upstream: string;
  tracking: string;
  worktree: string;
  protected: boolean;
}
export interface GitCommit {
  sha: string;
  parents: string[];
  author: string;
  timestamp: string;
  subject: string;
}
export interface Worktree {
  rootPath?: string;
  root: RootInfo;
  status: GitStatus;
  gitCommonIdentity: string;
}
export interface ReviewList {
  nextCursor?: number | null;
  items: {
    runId: number;
    objective: string;
    attempt: Pick<TaskAttempt, "id" | "taskId" | "attemptNumber" | "agent" | "status">;
  }[];
  bounded: boolean;
}
export interface HumanComment {
  id: number;
  body: string;
  decision: string;
  path: string | null;
  line: number | null;
  side: string | null;
  base: string;
  head: string;
  outdated: boolean;
  actor: string;
  createdAt: string;
}
export interface ReviewEvidence {
  run: { id: number; objective: string; status: string };
  task: { id: number; title: string; state: string };
  attempt: TaskAttempt;
  actualPaths: string[];
  patchHashVerified: boolean;
  sessions: {
    id: number;
    role: string;
    agent: string;
    status: string;
    reviewContext?: { capturedPatchSupplied: boolean; patchSha256: string };
  }[];
  humanComments: HumanComment[];
  integration: {
    id: number;
    task_id: number;
    status: string;
    before_sha: string;
    source_sha: string;
    candidate_sha: string | null;
    details: unknown;
  }[];
  handoffs: unknown[];
  authority: string;
}
