import { useState } from "react";
import type { TaskAttempt } from "../../../types";

export function AttemptEvidence({ attempt }: { attempt: TaskAttempt }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <details
      className="ws-attempt-evidence"
      onToggle={(event) => setExpanded(event.currentTarget.open)}
    >
      <summary>
        Task #{attempt.taskId} · attempt {attempt.attemptNumber} · {attempt.status}
      </summary>
      {expanded && (
        <>
          <p>
            {attempt.review
              ? `Reviewer: ${attempt.review.decision} — ${attempt.review.reason}`
              : "No review decision recorded."}
          </p>
          {attempt.error && <p>{attempt.error}</p>}
          <p className="ws-path">Task commit: {attempt.commitSha ?? "No commit recorded"}</p>
          <p>Captured task-worktree diff. The primary working tree is a separate version.</p>
          {attempt.evidence?.gitSnapshot && (
            <div aria-label="Rust-captured Git facts">
              <p className="ws-path">Base: {attempt.evidence.gitSnapshot.baseSha}</p>
              <p className="ws-path">Captured head: {attempt.evidence.gitSnapshot.headSha}</p>
              <p className="ws-path">Patch SHA-256: {attempt.evidence.gitSnapshot.patchSha256}</p>
              {attempt.evidence.gitSnapshot.patchTruncated && (
                <p role="status">
                  This captured patch is bounded. Inspect the retained worktree for the complete
                  change.
                </p>
              )}
            </div>
          )}
          {attempt.evidence?.diffPatch ? (
            <pre
              tabIndex={0}
              aria-label={`Captured diff for task ${attempt.taskId}, attempt ${attempt.attemptNumber}`}
            >
              {attempt.evidence.diffPatch}
            </pre>
          ) : (
            <p>No patch was captured for this attempt.</p>
          )}
        </>
      )}
    </details>
  );
}
