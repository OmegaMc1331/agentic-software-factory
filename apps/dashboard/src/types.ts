export type TaskState = "pending" | "ready" | "running" | "blocked" | "failed" | "completed";

export type TaskOperation =
  "planning" | "advisory" | "implement" | "verify" | "review" | "post_process";

export type ArtifactKind =
  "research" | "architecture" | "analysis" | "review" | "verification" | "documentation_context";

export interface RoleArtifact {
  id: number;
  runId: number;
  taskId: number | null;
  attemptId: number | null;
  role: string;
  operation: TaskOperation | null;
  kind: ArtifactKind;
  content: string;
  createdAt: string;
}

/** Derived "what kind of work is happening" stage shown in inspectors. */
export type WorkflowStage =
  | "planning"
  | "analysis"
  | "implementation"
  | "verification"
  | "review"
  | "post_processing"
  | "completed";

export interface StageStatus {
  key: string;
  label: string;
  total: number;
  completed: number;
  state: "completed" | "active" | "pending";
}

export const STAGE_META: Record<WorkflowStage, { label: string; short: string; color: string }> = {
  planning: { label: "Planning", short: "Plan", color: "#6b7280" },
  analysis: { label: "Analysis", short: "Analysis", color: "#0d9488" },
  implementation: { label: "Implementation", short: "Implement", color: "#2563eb" },
  verification: { label: "Verification", short: "Verify", color: "#7c3aed" },
  review: { label: "Review", short: "Review", color: "#b45309" },
  post_processing: { label: "Post-processing", short: "Docs", color: "#334155" },
  completed: { label: "Completed", short: "Done", color: "#16a34a" },
};

export function operationStage(operation: TaskOperation | null | undefined): WorkflowStage {
  switch (operation) {
    case "advisory":
      return "analysis";
    case "implement":
      return "implementation";
    case "verify":
      return "verification";
    case "review":
      return "review";
    case "post_process":
      return "post_processing";
    default:
      return "planning";
  }
}

export interface TaskCounts {
  pending: number;
  ready: number;
  running: number;
  blocked: number;
  failed: number;
  completed: number;
  total: number;
}

export interface RunSummary {
  id: number;
  objective: string;
  status: string;
  plannerAgent: string | null;
  createdAt: string;
  counts: TaskCounts;
}

export interface WorkflowTeam {
  planner: string;
  workers: string[];
  reviewers: string[];
  additional: Record<string, string[]>;
}

export interface Run {
  id: number;
  objective: string;
  status: string;
  plannerAgent: string | null;
  team: WorkflowTeam | null;
  createdAt: string;
  updatedAt: string;
}

export interface Task {
  id: number;
  runId: number;
  title: string;
  objective: string;
  acceptanceCriteria: string[];
  state: TaskState;
  position: number;
  dependencies: number[];
  worktreePath: string | null;
  role: string | null;
  operation: TaskOperation | null;
  agentOverride?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface RunDetail {
  run: Run;
  tasks: Task[];
  attempts: TaskAttempt[];
  sessions: AgentSession[];
  stages: StageStatus[];
  artifacts: RoleArtifact[];
  integration: RunIntegration;
}

/** Per-run integration branch (`factory/run-<id>`): head + integrated tasks. */
export interface RunIntegration {
  branch: string;
  head: string | null;
  integratedTasks: number[];
}

export function isImplementationOperation(operation: TaskOperation | null | undefined): boolean {
  return operation === "implement" || operation === "verify" || operation === "post_process";
}

/** Whether a task's approved work was merged into the run integration branch. */
export function isTaskIntegrated(
  task: Task,
  integration: RunIntegration | null | undefined
): boolean {
  return isTaskIntegratedIds(task.operation, task.id, integration);
}

/** Task-node variant of `isTaskIntegrated` for `TaskMeta` (graph) sources. */
export function isTaskIntegratedIds(
  operation: TaskOperation | null | undefined,
  taskId: number,
  integration: RunIntegration | null | undefined
): boolean {
  return (
    isImplementationOperation(operation) && (integration?.integratedTasks ?? []).includes(taskId)
  );
}

export type AgentKind =
  "codex" | "claude_code" | "open_code" | "gemini_cli" | "qwen_code" | "custom";

export type PromptTransport = "stdin" | "argument" | "disabled";

export type AgentResolutionStatus = "available" | "missing" | "broken";

export interface AgentEntry {
  kind?: AgentKind;
  command: string;
  args: string[];
  env: Record<string, string>;
  prompt_transport?: PromptTransport;
  interactive_args?: string[];
  capabilities?: string[];
}

export interface RoleDefinitionEntry {
  name?: string;
  description?: string;
  execution_class?: ExecutionClass;
  instructions?: string;
}

export interface RoleAssignmentEntry {
  role: string;
  agent: string;
  preferred?: boolean;
}

export interface RoutingConfigData {
  mode: RoutingModeValue;
  exploration: boolean;
}

export interface ConfigData {
  agents: Record<string, AgentEntry>;
  roles: Record<string, RoleDefinitionEntry>;
  role_assignments: RoleAssignmentEntry[];
  routing?: RoutingConfigData;
}

export type ExecutionClass = "planning" | "execution" | "review" | "advisory" | "post_process";

// --- Routing ------------------------------------------------------------------

export type RoutingModeValue = "round_robin" | "performance" | "manual";

export const ROUTING_MODE_LABELS: Record<RoutingModeValue, string> = {
  round_robin: "Round-robin (deterministic)",
  performance: "Performance-aware",
  manual: "Manual (preferred / pinned)",
};

export interface RoutingCandidateScore {
  agent: string;
  score: number | null;
  reliable: boolean;
  note: string;
}

/** What the router would do for a task right now (informational). */
export interface RoutingPreview {
  mode: RoutingModeValue;
  taskId: number;
  role: string | null;
  operation: TaskOperation | null;
  language: string | null;
  overrideAgent: string | null;
  likelyAgent: string | null;
  reason: string;
  candidates: RoutingCandidateScore[];
}

/** Durable audit record of one dispatch. */
export interface RoutingDecision {
  id: number;
  taskId: number;
  attemptId: number | null;
  mode: string;
  selectedAgent: string;
  role: string | null;
  operation: TaskOperation | null;
  language: string | null;
  candidateScores: RoutingCandidateScore[];
  reason: string;
  createdAt: string;
}

/** Whether an agent's metrics currently feed routing (Performance view). */
export interface RoutingUsage {
  mode: string;
  usedForRouting: boolean;
  note: string;
}

/** Policy preset names a role can select (what Factory permits, not instructions). */
export type RolePolicyPreset =
  "read_only" | "implementation" | "documentation" | "review" | "custom";

export interface RoleAssignmentInfo {
  agent: string;
  preferred: boolean;
}

/**
 * Effective policy of a role or agent, resolved by Factory Core. This is the
 * single policy view: the same resolution drives execution, validation,
 * session audit, and this display.
 */
export interface PolicyView {
  source: string;
  /** True when no policy is configured: legacy permissive mode. */
  permissive: boolean;
  filesystemMode: string;
  readScopes: string[];
  writeScopes: string[];
  denyWriteScopes: string[];
  commandsMode: string;
  commandsAllow: string[];
  commandsDeny: string[];
  network: string;
  /** Always "advisory": Factory cannot sandbox a launched process's network. */
  networkEnforcement: string;
  environmentMode: string;
  environmentAllowed: string[];
  environmentDenied: string[];
  gitAllowed: string[];
  gitDenied: string[];
}

export interface RoleInfo {
  id: string;
  name: string;
  kind: "core" | "custom";
  description: string;
  instructions: string;
  executionClass: ExecutionClass;
  assignments: RoleAssignmentInfo[];
  available: boolean;
  /** Effective permissions (always present in API responses). */
  permissions?: PolicyView;
  /** Configured policy preset, when set (null = no preset / permissive). */
  policyPreset?: string | null;
}

export const PIPELINE_ROLE_IDS = ["planner", "worker", "reviewer"] as const;

export function roleAgents(role: RoleInfo): string[] {
  return role.assignments.map((assignment) => assignment.agent);
}

export function preferredRoleAgents(role: RoleInfo | undefined): string[] {
  if (!role) return [];
  const preferred = role.assignments.filter((assignment) => assignment.preferred);
  return (preferred.length > 0 ? preferred : role.assignments).map(
    (assignment) => assignment.agent
  );
}

export interface AgentStatusInfo {
  name: string;
  command: string;
  args: string[];
  available: boolean;
  status?: AgentResolutionStatus;
  kind?: AgentKind;
  workflowAvailable?: boolean;
  interactiveAvailable?: boolean;
  resolvedExecutable?: string | null;
  resolutionError?: string | null;
  resolutionShim?: string | null;
  resolutionTarget?: string | null;
  resolutionKind?: string | null;
  pathEntriesChecked?: number;
  permissions?: PolicyView;
}

export type GraphNodeKind =
  "agent" | "role" | "run" | "task" | "group" | "note" | "github_issue" | "github_pr";
export type GraphEdgeKind =
  | "binds"
  | "plans"
  | "works"
  | "reviews"
  | "contains"
  | "depends"
  | "custom"
  | "membership"
  | "originates"
  | "delivers";

export interface AgentMeta {
  command: string;
  available: boolean;
  status?: AgentResolutionStatus;
  kind?: AgentKind;
  workflowAvailable?: boolean;
  interactiveAvailable?: boolean;
  resolvedExecutable?: string | null;
  resolutionError?: string | null;
  resolutionShim?: string | null;
  resolutionTarget?: string | null;
  resolutionKind?: string | null;
  pathEntriesChecked?: number;
  permissions?: PolicyView;
  roles: string[];
}

export function agentResolutionStatusLabel(status: AgentResolutionStatus | undefined): string {
  switch (status) {
    case "available":
      return "Resolved";
    case "broken":
      return "Invalid Windows executable";
    case "missing":
      return "Not found in Factory PATH";
    default:
      return "Unknown";
  }
}

export interface RoleMeta {
  id: string;
  name: string;
  kind: "core" | "custom";
  description: string;
  instructions: string;
  executionClass: ExecutionClass;
  assignments: RoleAssignmentInfo[];
  available: boolean;
  permissions?: PolicyView;
  policyPreset?: string | null;
}

export interface RunMeta {
  runId: number;
  objective: string;
  status: string;
  plannerAgent: string | null;
  team: WorkflowTeam | null;
  createdAt: string;
  counts: TaskCounts;
  /** GitHub origin, when the workflow was imported from an Issue. */
  github?: {
    issueNumber: number;
    issueUrl: string;
    issueTitle: string;
    repository: string;
  } | null;
  /** Delivery snapshot, when a delivery record exists. */
  delivery?: {
    state: string;
    prNumber: number | null;
    prUrl: string | null;
  } | null;
}

// --- GitHub -----------------------------------------------------------------

export type DeliveryState =
  "not_ready" | "ready" | "pushing" | "creating_pr" | "published" | "failed";

export interface GitHubIssueLink {
  provider: string;
  repository: string;
  issueNumber: number;
  issueUrl: string;
  issueTitle: string;
  issueBody: string;
  issueState: string;
  issueAuthor: string;
  issueLabels: string[];
  issueComments: { author: string; body: string }[];
  importedAt: string;
}

export interface PullRequestInfo {
  number: number;
  url: string;
  state: string;
  isDraft: boolean;
}

export interface GitHubRepoStatus {
  repository: string;
  remote: string;
  url: string;
  defaultBranch: string | null;
}

export interface GitHubStatus {
  connected: boolean;
  user: string | null;
  authError: string | null;
  remoteError: string | null;
  repository: GitHubRepoStatus | null;
}

export interface DeliveryReport {
  runId: number;
  state: DeliveryState;
  persistedState: DeliveryState;
  link: GitHubIssueLink | null;
  repository: GitHubRepoStatus | null;
  baseBranch: string | null;
  headBranch: string;
  integrationHead: string | null;
  localHead: string | null;
  pushedHead: string | null;
  pullRequest: PullRequestInfo | null;
  error: string | null;
  eligible: boolean;
  blockers: string[];
}

/** The persisted delivery record returned after `Create Pull Request`. */
export interface GitHubDeliveryRecord {
  runId: number;
  state: DeliveryState;
  repository: string | null;
  remote: string | null;
  baseBranch: string | null;
  headBranch: string;
  pushedHead: string | null;
  pullRequest: PullRequestInfo | null;
  error: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface PrPreview {
  runId: number;
  repository: string;
  base: string;
  head: string;
  title: string;
  body: string;
  draft: boolean;
  issueNumber: number | null;
  issueUrl: string | null;
  existing: PullRequestInfo | null;
  eligible: boolean;
  blockers: string[];
}

export interface GitHubIssueMeta {
  runId: number;
  number: number;
  repository: string;
  url: string;
  title: string;
  state: string;
  author: string;
  labels: string[];
}

export interface GitHubPrMeta {
  runId: number;
  number: number;
  url: string;
  state: string;
  isDraft: boolean;
}

export type AttemptStatus =
  | "running"
  | "reviewing"
  | "approved"
  | "changes_requested"
  | "failed"
  | "interrupted"
  | "cancelled";

export interface TaskEvidence {
  changedFiles: string[];
  diffSummary: string;
  commitSha: string | null;
  commands: string[];
  acceptanceCriteria: string[];
  workerExitCode: number | null;
  artifacts?: number[];
  diffPatch?: string | null;
}

export interface ReviewResult {
  decision: "approve" | "request_changes";
  reason: string;
  feedback: string[];
}

export interface TaskAttempt {
  id: number;
  taskId: number;
  attemptNumber: number;
  agent: string;
  role: string | null;
  operation: TaskOperation | null;
  status: AttemptStatus;
  startedAt: string;
  finishedAt: string | null;
  worktreePath: string;
  commitSha: string | null;
  exitCode: number | null;
  error: string | null;
  evidence: TaskEvidence | null;
  review: ReviewResult | null;
}

export interface TaskMeta {
  taskId: number;
  runId: number;
  objective: string;
  state: TaskState;
  position: number;
  dependencies: number[];
  acceptanceCriteria: string[];
  worktreePath: string | null;
  role: string | null;
  operation: TaskOperation | null;
  currentAttempt: TaskAttempt | null;
}

export interface GroupMeta {
  visualOnly: true;
}

export interface NoteMeta {
  text: string;
  visualOnly: true;
}

export interface GraphNode {
  id: string;
  kind: GraphNodeKind;
  label: string;
  meta:
    | AgentMeta
    | RoleMeta
    | RunMeta
    | TaskMeta
    | GroupMeta
    | NoteMeta
    | GitHubIssueMeta
    | GitHubPrMeta;
}

export interface GraphEdge {
  id: string;
  source: string;
  target: string;
  kind: GraphEdgeKind;
  editable: boolean;
  semantic: "configuration" | "execution" | "system" | "visual";
}

export interface GraphMetadata {
  runs: number;
  tasks: number;
  agents: number;
  missingAgents: number;
  roles: number;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
  metadata: GraphMetadata;
}

export interface GraphPosition {
  x: number;
  y: number;
}

export interface WorkspaceNode {
  id: string;
  kind: "group" | "note";
  label: string;
  text?: string;
}

export interface WorkspaceEdge {
  id: string;
  source: string;
  target: string;
  kind: "custom" | "membership";
}

export interface GraphWorkspace {
  version: 1;
  nodes: Record<string, GraphPosition>;
  customNodes: WorkspaceNode[];
  edges: WorkspaceEdge[];
  warning?: string;
}

/** Which policy applied to an automated session (no secret values). */
export interface SessionPolicyAudit {
  source: string;
  filesystem: string;
  network: string;
  environment: string;
  writeScopes: string[];
}

export interface AgentSession {
  id: number;
  runId: number | null;
  taskId: number | null;
  attemptId: number | null;
  role: string;
  operation: TaskOperation | null;
  agent: string;
  mode?: "automated" | "interactive";
  command: string;
  status: string;
  startedAt: string;
  finishedAt: string | null;
  exitCode: number | null;
  durationMs: number | null;
  stdout: string | null;
  stderr: string | null;
  policyAudit?: SessionPolicyAudit | null;
  workingDirectory: string;
  interactive: boolean;
}

export function agentMeta(node: GraphNode): AgentMeta {
  return node.meta as AgentMeta;
}

export function roleMeta(node: GraphNode): RoleMeta {
  return node.meta as RoleMeta;
}

export function runMeta(node: GraphNode): RunMeta {
  return node.meta as RunMeta;
}

export function taskMeta(node: GraphNode): TaskMeta {
  return node.meta as TaskMeta;
}

export function githubIssueMeta(node: GraphNode): GitHubIssueMeta {
  return node.meta as GitHubIssueMeta;
}

export function githubPrMeta(node: GraphNode): GitHubPrMeta {
  return node.meta as GitHubPrMeta;
}

export interface AgentActivity {
  runId: number;
  taskId: number | null;
}

function runNodeId(node: GraphNode): number {
  return Number(node.id.slice("run:".length)) || 0;
}

export function agentActivity(
  agentId: string,
  nodesById: Map<string, GraphNode>,
  edges: GraphEdge[]
): AgentActivity | null {
  const roles = new Set<string>();
  for (const edge of edges) {
    if (edge.kind === "binds" && edge.target === agentId) roles.add(edge.source);
  }
  const relevant = (id: string) => id === agentId || roles.has(id);

  const direct = edges.find(
    (edge) => edge.source === agentId && (edge.kind === "works" || edge.kind === "reviews")
  );
  if (direct) {
    const task = nodesById.get(direct.target);
    if (task?.kind === "task") {
      const meta = taskMeta(task);
      return { runId: meta.runId, taskId: meta.taskId };
    }
  }

  const activeRuns: GraphNode[] = [];
  for (const edge of edges) {
    if (edge.kind !== "plans" || !relevant(edge.target)) continue;
    const runNode = nodesById.get(edge.source);
    if (runNode && runNode.kind === "run" && runMeta(runNode).status === "planning") {
      activeRuns.push(runNode);
    }
  }
  if (activeRuns.length === 0) return null;
  activeRuns.sort((a, b) => runNodeId(b) - runNodeId(a));
  const runId = runNodeId(activeRuns[0]);

  let taskId: number | null = null;
  for (const node of nodesById.values()) {
    if (node.kind !== "task") continue;
    const meta = taskMeta(node);
    if (
      meta.runId === runId &&
      meta.state === "running" &&
      (taskId === null || meta.taskId > taskId)
    ) {
      taskId = meta.taskId;
    }
  }
  return { runId, taskId };
}

// --- Performance evaluation --------------------------------------------------

export type PerformanceWindowKey = "all_time" | "last_30_days" | "last_7_days";

export interface RateStats {
  successes: number;
  total: number;
  rate: number | null;
  intervalLow: number | null;
  intervalHigh: number | null;
  reliable: boolean;
}

export interface DurationStats {
  samples: number;
  medianMs: number | null;
  p95Ms: number | null;
  approximateSamples: number;
  reliable: boolean;
}

export interface OutcomeCounts {
  approved: number;
  firstPassApproved: number;
  changesRequested: number;
  agentFailed: number;
  integrationConflict: number;
  cancelled: number;
  interrupted: number;
  policyBlocked: number;
  configurationError: number;
  inProgress: number;
}

export interface IntegrationStats {
  clean: number;
  rebased: number;
  conflict: number;
  cleanRate: RateStats;
  conflictRate: RateStats;
}

export interface AgentMetrics {
  tasksAttempted: number;
  attempts: number;
  attemptsPerTask: number | null;
  avgAttemptsPerSuccessful: number | null;
  qualifyingTasks: number;
  outcomeCounts: OutcomeCounts;
  firstPassApproval: RateStats;
  eventualApproval: RateStats;
  requestChanges: RateStats;
  retryRate: RateStats;
  terminalFailure: RateStats;
  executionDuration: DurationStats;
  reviewDuration: DurationStats;
  totalDuration: DurationStats;
  integration: IntegrationStats;
}

export interface AgentPerformanceSummary {
  agent: string;
  metrics: AgentMetrics;
}

export interface PerformanceBreakdownEntry {
  dimension: string;
  key: string;
  metrics: AgentMetrics;
}

export interface ReasonCount {
  reason: string;
  count: number;
}

export interface TrendWindow {
  label: string;
  firstPass: RateStats;
  medianExecutionMs: number | null;
}

export interface TrendComparison {
  current: TrendWindow;
  previous: TrendWindow;
  firstPassDeltaPp: number | null;
}

export interface TrendSummary {
  recent10: TrendWindow;
  recent25: TrendWindow;
  weekly: TrendComparison | null;
}

export interface AgentPerformanceDetail {
  summary: AgentPerformanceSummary;
  byRole: PerformanceBreakdownEntry[];
  byOperation: PerformanceBreakdownEntry[];
  byLanguage: PerformanceBreakdownEntry[];
  trend: TrendSummary;
  reworkReasons: ReasonCount[];
  failureReasons: ReasonCount[];
  routing?: RoutingUsage;
}

export interface PerformanceFacets {
  roles: string[];
  operations: string[];
  languages: string[];
}

export interface PerformanceOverview {
  window: PerformanceWindowKey;
  agents: AgentPerformanceSummary[];
  facets: PerformanceFacets;
}

export interface PerformanceFilters {
  window?: string;
  role?: string;
  operation?: string;
  language?: string;
}
