import type {
  AgentData,
  ChatMessage,
  FileReference,
  MissionData,
  TestData,
} from "../../design-system/patterns/domain";
import type { NotificationData } from "../../design-system/components/Application";

export type Scenario =
  "empty" | "conversation" | "running" | "attention" | "code" | "review" | "long";
export const scenarios: { id: Scenario; title: string; label: string }[] = [
  { id: "empty", title: "UX-01 · Empty workspace", label: "Empty" },
  { id: "conversation", title: "UX-02 · Active conversation", label: "Conversation" },
  { id: "running", title: "UX-03 · Mission running", label: "Running mission" },
  { id: "attention", title: "UX-04 · Attention required", label: "Attention" },
  { id: "code", title: "UX-05 · Code workspace", label: "Code" },
  { id: "review", title: "UX-06 · Source control and review", label: "Review" },
  { id: "long", title: "UX-10 · Long content", label: "Long content" },
];

export type FixtureFile = FileReference & { text: string; before: string };
const original =
  "use crate::session::{Checkpoint, Session};\n\npub fn restore(session: &Session) -> Checkpoint {\n    session.last_checkpoint().unwrap()\n}\n";
const revised =
  "use crate::session::{Checkpoint, CheckpointError, Session};\n\npub fn restore(session: &Session) -> Result<Checkpoint, CheckpointError> {\n    let checkpoint = session.last_checkpoint()\n        .ok_or(CheckpointError::Missing)?;\n\n    if checkpoint.owner != session.id {\n        return Err(CheckpointError::OwnerMismatch);\n    }\n\n    Ok(checkpoint)\n}\n";
export const fixtureFiles: FixtureFile[] = [
  {
    path: "src/runtime/checkpoints.rs",
    language: "Rust",
    text: revised,
    before: original,
    added: 12,
    removed: 3,
    origin: "agent",
    review: "pending",
  },
  {
    path: "src/runtime/recovery.rs",
    language: "Rust",
    text: "pub enum RecoveryState {\n    Ready,\n    AwaitingPermission,\n    Suspended,\n}\n\npub fn can_resume(state: &RecoveryState) -> bool {\n    matches!(state, RecoveryState::Ready)\n}\n",
    before: "pub fn can_resume() -> bool {\n    true\n}\n",
    added: 9,
    removed: 2,
    origin: "agent",
    review: "pending",
  },
  {
    path: "src/ui/session.ts",
    language: "TypeScript",
    text: "export type SessionState = 'active' | 'suspended';\n\nexport function describeSession(state: SessionState): string {\n  return state === 'active' ? 'Work continues' : 'Checkpoint preserved';\n}\n",
    before: "export const sessionLabel = 'Active';\n",
    added: 5,
    removed: 1,
    origin: "manual",
    review: "pending",
  },
  {
    path: "tests/test_recovery.py",
    language: "Python",
    text: "from recovery import can_resume\n\ndef test_checkpoint_owner_is_required():\n    checkpoint = {'owner': 'session-01', 'verified': True}\n    assert can_resume(checkpoint, 'session-01')\n    assert not can_resume(checkpoint, 'another-session')\n",
    before: "def test_recovery():\n    assert True\n",
    added: 6,
    removed: 2,
    origin: "agent",
    review: "pending",
  },
  {
    path: "docs/recovery-contract.md",
    language: "Markdown",
    text: "# Recovery contract\n\nA checkpoint belongs to one project and session.\nRevalidate its owner and permissions before resuming.\nAn uncertain external operation requires reconciliation.\n",
    before: "# Recovery contract\n",
    added: 4,
    removed: 0,
    origin: "manual",
    review: "accepted",
  },
  {
    path: "Cargo.toml",
    language: "Text",
    text: '[package]\nname = "atlas-fixture"\nversion = "0.1.0"\nedition = "2021"\n',
    before: "",
    added: 0,
    removed: 0,
    origin: "manual",
    review: "accepted",
  },
];

export function filesFor(scenario: Scenario): FixtureFile[] {
  if (scenario !== "long") return fixtureFiles;
  return [
    ...fixtureFiles,
    ...Array.from({ length: 2000 }, (_, index): FixtureFile => ({
      path: `src/generated/group-${String(Math.floor(index / 100)).padStart(2, "0")}/checkpoint_ownership_and_reconciliation_policy_${String(index).padStart(4, "0")}.rs`,
      language: "Rust",
      text: `pub const CHECKPOINT_${index}: usize = ${index};\n`,
      before: "",
      added: 1,
      removed: 0,
      origin: "agent",
      review: "pending",
    })),
  ];
}

const baseMessages: ChatMessage[] = [
  {
    id: "u-01",
    role: "user",
    author: "You",
    time: "09:41",
    body: "Make session recovery safer. A checkpoint from another session must never be resumed accidentally. Keep the change small and cover the ownership check.",
    provenance: "fixture",
  },
  {
    id: "o-01",
    role: "orchestrator",
    author: "Orchestrator",
    time: "09:42",
    body: "The checkpoint records an owner, but the restore path does not check it. I’ll add that validation and keep a missing checkpoint recoverable. The existing permission check stays in place.",
    files: [fixtureFiles[0]],
    provenance: "fixture",
  },
  {
    id: "u-02",
    role: "user",
    author: "You",
    time: "09:44",
    body: "Also keep the error useful. I want to know which session owns the checkpoint before deciding what to do next.",
    provenance: "fixture",
  },
  {
    id: "o-02",
    role: "orchestrator",
    author: "Orchestrator",
    time: "09:46",
    body: "The ownership guard and the recovery error are ready to review. The tests cover the matching owner, a different owner and a missing checkpoint. Publication remains a separate decision.",
    files: [fixtureFiles[0], fixtureFiles[1], fixtureFiles[3]],
    result: { title: "Checkpoint ownership is explicit", fileCount: 4, testsPassed: 9 },
    provenance: "fixture",
  },
];

export function messagesFor(scenario: Scenario): ChatMessage[] {
  if (scenario === "empty") return [];
  if (scenario === "running" || scenario === "attention") return baseMessages.slice(0, 3);
  if (scenario !== "long") return baseMessages;
  return Array.from({ length: 120 }, (_, index): ChatMessage => ({
    id: `long-${index}`,
    role: index % 2 ? "orchestrator" : "user",
    author: index % 2 ? "Orchestrator" : "You",
    time: "09:46",
    body:
      index === 119
        ? "Checkpoint policy and recovery evidence\n\n" +
          "Preserve session identity, revalidate permissions, and reconcile uncertain external operations before retrying. ".repeat(
            110
          )
        : `Review item ${index + 1}: preserve checkpoint ownership and the verified state of the session.`,
    provenance: "fixture",
    files: index === 119 ? [filesFor("long")[130]] : undefined,
  }));
}

export function missionFor(scenario: Scenario): MissionData {
  const active = scenario === "running" || scenario === "attention";
  return {
    id: "mission-014",
    title:
      scenario === "long"
        ? "Preserve checkpoint ownership across concurrent sessions and worktree reconciliation without repeating uncertain external publication operations"
        : "Validate checkpoint ownership",
    status:
      scenario === "attention"
        ? "attention"
        : active
          ? "running"
          : scenario === "empty"
            ? "idle"
            : "completed",
    expectedResult: "A session can resume only from its own verified checkpoint.",
    provenance: "fixture",
    checkpoint: { id: "cp-014", commit: "a42bd91", verified: true },
    steps: [
      {
        id: "inspect",
        title: "Inspect the restore path",
        status: "completed",
        detail: "Checkpoint and session identity",
      },
      {
        id: "implement",
        title: "Add the ownership guard",
        status: "completed",
        detail: "Changes isolated for review",
      },
      {
        id: "test",
        title: "Verify recovery behavior",
        status: active ? (scenario === "attention" ? "attention" : "running") : "completed",
        detail:
          scenario === "attention"
            ? "The assigned agent is unavailable."
            : "Ownership and missing-checkpoint cases",
      },
      { id: "review", title: "Review changes and evidence", status: active ? "idle" : "completed" },
      {
        id: "integrate",
        title: "Prepare local integration",
        status: active ? "idle" : "completed",
      },
    ],
  };
}

export const agents: AgentData[] = [
  {
    id: "codex",
    name: "Codex",
    role: "Implementation",
    model: "Project CLI profile",
    status: "working",
    task: "Verify checkpoint ownership",
    permissions: ["Project files", "Local tests"],
  },
  {
    id: "claude",
    name: "Claude Code",
    role: "Review",
    model: "Project CLI profile",
    status: "available",
    task: "Review error paths",
    permissions: ["Read project files", "Local tests", "Review artifacts"],
  },
  {
    id: "gemini",
    name: "Gemini CLI",
    role: "Diagnostics",
    model: "Project CLI profile",
    status: "available",
    task: "No assigned task",
    permissions: ["Read project files"],
  },
];
export const tests: TestData[] = [
  {
    name: "Checkpoint ownership",
    status: "passed",
    duration: "0.12 s",
    command: "cargo test checkpoint_owner",
    provenance: "fixture",
  },
  {
    name: "Missing checkpoint",
    status: "passed",
    duration: "0.08 s",
    command: "cargo test missing_checkpoint",
    provenance: "fixture",
  },
  {
    name: "Permission revalidation",
    status: "passed",
    duration: "0.16 s",
    command: "cargo test resume_permissions",
    provenance: "fixture",
  },
];
export const notifications: NotificationData[] = [
  {
    id: "review-ready",
    title: "Changes ready for review",
    detail: "The checkpoint ownership fixture includes four changed files and a test record.",
    severity: "success",
    createdAt: "Fixture · 09:46",
  },
  {
    id: "permission",
    title: "Publication requires permission",
    detail: "Review local changes before authorizing an external operation.",
    severity: "info",
    createdAt: "Fixture · 09:44",
  },
];
