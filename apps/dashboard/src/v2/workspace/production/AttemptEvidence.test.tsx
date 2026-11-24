import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { TaskAttempt } from "../../../types";
import { AttemptEvidence } from "./AttemptEvidence";

afterEach(cleanup);
const attempt: TaskAttempt = {
  id: 8,
  taskId: 3,
  attemptNumber: 2,
  agent: "worker",
  role: "worker",
  operation: "implement",
  status: "approved",
  startedAt: "2026-10-30",
  finishedAt: "2026-10-30",
  worktreePath: "task-root",
  commitSha: "abc123",
  exitCode: 0,
  error: null,
  evidence: {
    changedFiles: ["src/a.py"],
    diffSummary: "1 file",
    commitSha: "abc123",
    commands: [],
    acceptanceCriteria: [],
    workerExitCode: 0,
    diffPatch: "-return n + 1\n+return n\n<script>alert(1)</script>",
  },
  review: { decision: "approve", reason: "Checked the captured implementation", feedback: [] },
};

it("reveals the recorded patch and review as text only when expanded", () => {
  const { container } = render(<AttemptEvidence attempt={attempt} />);
  expect(screen.queryByLabelText("Captured diff for task 3, attempt 2")).toBeNull();
  const disclosure = container.querySelector("details")!;
  disclosure.open = true;
  fireEvent(disclosure, new Event("toggle"));
  expect(screen.getByLabelText("Captured diff for task 3, attempt 2").textContent).toContain(
    "+return n"
  );
  expect(screen.getByText(/Reviewer: approve/).textContent).toContain(
    "Checked the captured implementation"
  );
  expect(container.querySelector("script")).toBeNull();
  expect(screen.getByText(/primary working tree is a separate version/)).toBeTruthy();
});

it("does not imply a patch or review exists for an empty failed attempt", () => {
  const { container } = render(
    <AttemptEvidence
      attempt={{ ...attempt, status: "failed", evidence: null, review: null, commitSha: null }}
    />
  );
  const disclosure = container.querySelector("details")!;
  disclosure.open = true;
  fireEvent(disclosure, new Event("toggle"));
  expect(screen.getByText("No patch was captured for this attempt.")).toBeTruthy();
  expect(screen.getByText("No review decision recorded.")).toBeTruthy();
});
