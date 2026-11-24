import type { FileRef } from "../files/types";
export interface Conversation {
  id: string;
  project_id: string;
  title: string;
  status: string;
  revision: number;
  created_at: string;
  updated_at: string;
}
export interface ContextReference {
  file: FileRef;
  revision: string;
  buffer?: string;
  selection?: [number, number];
  diagnostic?: string;
}
export interface Message {
  id: number;
  conversation_id: string;
  role: string;
  content: string;
  status: string;
  run_id: number | null;
  parent_message_id: number | null;
  provenance: {
    source?: string;
    context?: Array<{
      file: FileRef;
      state: string;
      revision: string;
      content: string;
      read_only?: boolean;
    }>;
    agent_session_id?: number;
    memory_references?: string[];
    snapshot?: Mission;
    error?: string | boolean;
    command_id?: number;
  };
  created_at: string;
}
export interface Command {
  id: number;
  conversation_id: string;
  message_id: number;
  response_id: number;
  state: string;
  run_id: number | null;
  key: string;
}
export interface ConversationEvent {
  schema_version: 1;
  id: number;
  conversation_id: string;
  event_type: string;
  payload: Record<string, unknown>;
}
export interface PlannedTask {
  id: string;
  title: string;
  objective: string;
  dependencies: string[];
  acceptanceCriteria: string[];
  role: string;
  operation?: string;
}
export interface Plan {
  objective: string;
  tasks: PlannedTask[];
}
export interface MissionRequest {
  objective: string;
  scope: string;
  constraints: string;
  acceptance_criteria: string[];
  team?: unknown;
  recipe?: string;
  plan?: Plan;
}
export interface Submission {
  key: string;
  workspace_session_id: string;
  content: string;
  context: ContextReference[];
  mission?: MissionRequest;
}
export interface Mission {
  lifecycle?: import("../lifecycle/types").MissionLifecycle | null;
  run_id: number;
  objective: string;
  status: string;
  completed: number;
  total: number;
  terminal: boolean;
  current?: { id: number; title: string; role: string };
  active_tasks?: Array<{ id: number; title: string; role?: string; stage: string }>;
  plan_revision?: number;
  plan_revisions?: Array<{
    revision: number;
    source: string;
    reason: string | null;
    plannerSessionId: number | null;
    createdAt: string;
    tasks: Array<{ id: number; title: string; state: string; dependencies: number[] }>;
  }>;
  integrations?: Array<{
    id: number;
    taskId: number;
    attemptId: number;
    status: string;
    beforeSha: string;
    sourceSha: string;
    candidateSha: string | null;
  }>;
  tasks: Array<{ id: number; title: string; state: string; role?: string; dependencies: number[] }>;
  sessions: Array<{
    id: number;
    agent: string;
    role: string;
    status: string;
    task_id: number | null;
  }>;
  attempts: Array<{
    id: number;
    task_id: number;
    status: string;
    exit_code: number | null;
    error: string | null;
    commands?: string[];
    review?: { decision: string };
  }>;
  changed_files: string[];
  integration_sha: string | null;
  artifacts: Array<{ id: number; kind: string; task_id: number }>;
}
