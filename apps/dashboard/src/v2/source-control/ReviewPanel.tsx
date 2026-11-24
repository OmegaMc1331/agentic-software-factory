import { useState } from "react";
import { Button, Input, Select, Textarea } from "../design-system/primitives/Controls";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import type { WorkspaceView } from "../workspace/production/model";
import { useSourceData, sourceRequest } from "./client";
import { SourceDiff } from "./SourceControlPanel";
import type { ReviewEvidence, ReviewList, Worktree } from "./types";
import "./source-control.css";

export function ReviewPanel({
  view,
  dock,
  model,
}: {
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
}) {
  const [version, setVersion] = useState(0);
  const [attemptId, setAttemptId] = useState(view.state.cursor ?? 0);
  const [before, setBefore] = useState<number>(),
    [pages, setPages] = useState<(number | undefined)[]>([]);
  const list = useSourceData<ReviewList>(
    view.projectId,
    `reviews?${view.resourceId ? `runId=${view.resourceId}&` : ""}${before ? `before=${before}` : ""}`,
    version
  );
  const detail = useSourceData<ReviewEvidence>(
    view.projectId,
    `reviews/${attemptId}`,
    version,
    attemptId > 0
  );
  const roots = useSourceData<Worktree[]>(view.projectId, "worktrees", version);
  const [path, setPath] = useState(""),
    [body, setBody] = useState(""),
    [decision, setDecision] = useState("comment"),
    [line, setLine] = useState("");
  const [error, setError] = useState(""),
    [notice, setNotice] = useState(""),
    [busy, setBusy] = useState(false);
  const evidence = detail.value;
  const snapshot = evidence?.attempt.evidence?.gitSnapshot;
  const root = roots.value?.find((item) => !item.root.read_only);
  const conflictRecovered = (id: number) =>
    evidence?.run.status === "completed" &&
    evidence.integration.some(
      (record) => record.id > id && record.status === "published" && record.candidate_sha
    );
  const addComment = async () => {
    if (!snapshot) return;
    setBusy(true);
    setError("");
    try {
      await sourceRequest(view.projectId, `reviews/${attemptId}/comments`, {
        key: crypto.randomUUID(),
        anchor: {
          base: snapshot.baseSha,
          head: snapshot.headSha,
          path: path || null,
          line: line ? Number(line) : null,
          side: line ? "head" : null,
        },
        decision,
        body,
      });
      setBody("");
      setNotice(
        "Human feedback saved. Change requests fence pending publication; automated review and checks remain required."
      );
      setVersion((value) => value + 1);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="asf-source sc-review">
      <header className="sc-toolbar">
        <h2>Review workspace</h2>
        <Button onClick={() => setVersion((value) => value + 1)}>Refresh reviews</Button>
      </header>
      {list.error && (
        <InlineAlert tone="danger" title="Reviews unavailable">
          {list.error}
        </InlineAlert>
      )}
      <Select
        label="Recorded task attempt"
        value={attemptId || ""}
        onChange={(event) => {
          const id = Number(event.target.value);
          setAttemptId(id);
          setPath("");
          model.updateView(dock.sessionId, view.id, { cursor: id });
        }}
      >
        <option value="">Choose an actual attempt</option>
        {list.value?.items.map((item) => (
          <option key={item.attempt.id} value={item.attempt.id}>
            Mission {item.runId} · task {item.attempt.taskId} · attempt {item.attempt.attemptNumber}{" "}
            · {item.attempt.agent} · {item.attempt.status}
          </option>
        ))}
      </Select>
      <div className="sc-toolbar">
        <Button
          disabled={!pages.length}
          onClick={() => {
            setBefore(pages[pages.length - 1]);
            setPages((current) => current.slice(0, -1));
          }}
        >
          Previous attempts
        </Button>
        <Button
          disabled={!list.value?.nextCursor}
          onClick={() => {
            setPages((current) => [...current, before]);
            setBefore(list.value?.nextCursor ?? undefined);
          }}
        >
          Older attempts
        </Button>
      </div>
      {detail.error && (
        <InlineAlert tone="danger" title="Evidence unavailable">
          {detail.error}
        </InlineAlert>
      )}
      {attemptId > 0 && !evidence && !detail.error && (
        <LoadingIndicator label="Verifying recorded review evidence" />
      )}
      {evidence && (
        <>
          <h3>{evidence.task.title}</h3>
          <p>{evidence.run.objective}</p>
          <div className="sc-status">
            <span>Attempt: {evidence.attempt.status}</span>
            <span>Reviewer: {evidence.attempt.review?.decision ?? "No recorded decision"}</span>
            <Button
              onClick={() =>
                dock.open("progress", { projectId: view.projectId, resourceId: evidence.run.id })
              }
            >
              Open mission
            </Button>
          </div>
          <p>{evidence.attempt.review?.reason}</p>
          <p>{evidence.authority}</p>
          <div className="sc-version">
            <span>Base: {snapshot?.baseSha ?? "Not recorded"}</span>
            <span>Head: {snapshot?.headSha ?? evidence.attempt.commitSha ?? "Not recorded"}</span>
          </div>
          <p>
            Captured patch hash:{" "}
            {evidence.patchHashVerified
              ? "verified against persisted evidence"
              : "not verified / unavailable"}
            {snapshot?.patchTruncated ? " · captured patch is incomplete" : ""}
          </p>
          <ul className="sc-file-list">
            {evidence.actualPaths.map((name) => (
              <li key={name}>
                <Button onClick={() => setPath(name)}>{name}</Button>
                {root && (
                  <Button
                    onClick={() =>
                      void dock.openFile({
                        project_id: view.projectId,
                        root_id: root.root.id,
                        path: name,
                      })
                    }
                  >
                    Open primary source
                  </Button>
                )}
              </li>
            ))}
          </ul>
          {path && snapshot && root && (
            <SourceDiff
              projectId={view.projectId}
              rootId={root.root.id}
              path={path}
              comparison="commits"
              base={snapshot.baseSha}
              target={snapshot.headSha}
            />
          )}
          <h3>Reported validation commands</h3>
          <ul>
            {evidence.attempt.evidence?.commands.map((command, index) => (
              <li key={index}>
                <code>{command}</code>
              </li>
            ))}
          </ul>
          <p>These are persisted reports. Independent integration checks are shown separately.</p>
          <h3>Agent provenance</h3>
          <ul>
            {evidence.sessions.map((session) => (
              <li key={session.id}>
                <Button
                  onClick={() =>
                    dock.open("console", { projectId: view.projectId, resourceId: session.id })
                  }
                >
                  Session {session.id} · {session.agent} · {session.role} · {session.status}
                </Button>
                {session.reviewContext && (
                  <p>
                    Captured diff supplied:{" "}
                    {session.reviewContext.capturedPatchSupplied ? "yes" : "not confirmed"} ·{" "}
                    {session.reviewContext.patchSha256}
                  </p>
                )}
              </li>
            ))}
          </ul>
          <details>
            <summary>Agent handoff history · {evidence.handoffs.length}</summary>
            <pre>{JSON.stringify(evidence.handoffs, null, 2)}</pre>
          </details>
          <h3>Integration journal</h3>
          {evidence.integration.length ? (
            evidence.integration.map((record) => (
              <details key={record.id} open={record.status === "conflict"}>
                <summary>
                  Task {record.task_id} · {record.status} ·{" "}
                  {record.candidate_sha?.slice(0, 12) ?? "No published candidate"}
                </summary>
                <div className="sc-version">
                  <span>Previous: {record.before_sha}</span>
                  <span>Source: {record.source_sha}</span>
                </div>
                <pre>{JSON.stringify(record.details, null, 2)}</pre>
                {record.status === "conflict" && (
                  <InlineAlert
                    tone="warning"
                    title={
                      conflictRecovered(record.id)
                        ? "Historical conflict retained"
                        : "Coordinator recovery required"
                    }
                  >
                    {conflictRecovered(record.id) &&
                      "The mission completed with a later verified integration. "}
                    Both outputs remain preserved. Open the mission to inspect the configured
                    Planner repair/replan path. This view does not select a merge side or mutate its
                    worktree.
                  </InlineAlert>
                )}
              </details>
            ))
          ) : (
            <p>No integration journal exists for this mission.</p>
          )}
          <h3>Human feedback</h3>
          {evidence.humanComments.map((comment) => (
            <article key={comment.id} className="sc-comment">
              <strong>
                {comment.decision} · {comment.actor}
                {comment.outdated ? " · outdated diff" : ""}
              </strong>
              {comment.path && (
                <Button
                  onClick={() => {
                    setPath(comment.path ?? "");
                    setLine(comment.line?.toString() ?? "");
                  }}
                >
                  {comment.path}
                  {comment.line ? `:${comment.line}` : ""}
                </Button>
              )}
              <p>{comment.body}</p>
              <small>
                {comment.createdAt} · {comment.head.slice(0, 12)}
              </small>
            </article>
          ))}
          <Select
            label="Feedback type"
            value={decision}
            onChange={(event) => setDecision(event.target.value)}
          >
            <option value="comment">Comment</option>
            <option value="request_changes">Request changes</option>
            <option value="approve_annotation">Resolve human block / approval annotation</option>
          </Select>
          <Input
            label="Line in selected head file (optional)"
            type="number"
            min={1}
            max={100000}
            value={line}
            onChange={(event) => setLine(event.target.value)}
            disabled={!path}
          />
          <Textarea
            label="Human review comment"
            value={body}
            maxLength={8192}
            rows={3}
            onChange={(event) => setBody(event.target.value)}
          />
          <Button
            disabled={!snapshot || !body.trim()}
            loading={busy}
            onClick={() => void addComment()}
          >
            Save human feedback
          </Button>
          {error && (
            <InlineAlert tone="danger" title="Feedback was not saved">
              {error}
            </InlineAlert>
          )}
          <p role="status">{notice}</p>
        </>
      )}
    </div>
  );
}
