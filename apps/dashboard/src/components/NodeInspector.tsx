import { useEffect, useMemo, useState } from "react";
import type { GraphEdge, GraphNode } from "../types";
import { agentMeta, agentResolutionStatusLabel, runMeta, taskMeta } from "../types";
import { STATE_META } from "../state";
import { connectionKind } from "../graphWorkspace";

function Frame({
  title,
  kind,
  onClose,
  children,
}: {
  title: string;
  kind: string;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <aside className="net-inspector">
      <div className="inspector-header">
        <span className="inspector-kind">{kind}</span>
        <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
          x
        </button>
      </div>
      <h3 className="inspector-title">{title}</h3>
      <div className="inspector-body">{children}</div>
    </aside>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="inspector-row">
      <span className="inspector-label">{label}</span>
      <span className="inspector-value">{children}</span>
    </div>
  );
}

export function NodeInspector({
  node,
  edge,
  nodesById,
  onClose,
  onDelete,
  onConnect,
  onRetry,
}: {
  node: GraphNode | null;
  edge: GraphEdge | null;
  nodesById: Map<string, GraphNode>;
  onClose: () => void;
  onDelete: () => void;
  onConnect: (targetId: string) => void;
  onRetry?: (taskId: number) => void;
}) {
  const [target, setTarget] = useState("");
  useEffect(() => setTarget(""), [node?.id]);
  const targets = useMemo(
    () =>
      node
        ? Array.from(nodesById.values())
            .filter((candidate) => connectionKind(node, candidate) !== null)
            .sort((a, b) => a.label.localeCompare(b.label))
        : [],
    [node, nodesById]
  );

  if (edge) {
    return (
      <Frame title={edge.kind} kind="Connection" onClose={onClose}>
        <Row label="Source">
          <code>{nodesById.get(edge.source)?.label ?? edge.source}</code>
        </Row>
        <Row label="Target">
          <code>{nodesById.get(edge.target)?.label ?? edge.target}</code>
        </Row>
        <Row label="Meaning">{edge.semantic}</Row>
        <Row label="Editing">{edge.editable ? "Editable" : "Read-only Factory state"}</Row>
        {edge.editable && (
          <button className="button inspector-delete" onClick={onDelete}>
            Delete connection
          </button>
        )}
      </Frame>
    );
  }

  if (!node) {
    return (
      <aside className="net-inspector net-inspector--empty">
        <p className="inspector-hint">Select a node or connection to inspect it.</p>
        <p className="inspector-hint">Drag from a visible handle to create a supported link.</p>
      </aside>
    );
  }

  let details: React.ReactNode;
  if (node.kind === "agent") {
    const meta = agentMeta(node);
    details = (
      <>
        <Row label="Command">
          <code>{meta.command}</code>
        </Row>
        <Row label="Status">
          {meta.available
            ? "Available"
            : meta.status === "broken"
              ? "Broken installation"
              : "Missing"}
        </Row>
        <Row label="Workflow">{meta.workflowAvailable ? "available" : "unavailable"}</Row>
        <Row label="Interactive">{meta.interactiveAvailable ? "available" : "unavailable"}</Row>
        <details className="agent-resolution-details">
          <summary>Resolution details</summary>
          <div className="inspector-row">
            <span className="inspector-label">Status</span>
            <span className="inspector-value">{agentResolutionStatusLabel(meta.status)}</span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Shim</span>
            <span className="inspector-value">
              {meta.resolutionShim ? <code>{meta.resolutionShim}</code> : "—"}
            </span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Target</span>
            <span className="inspector-value">
              {meta.resolutionTarget ? <code>{meta.resolutionTarget}</code> : "—"}
            </span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Resolver kind</span>
            <span className="inspector-value">
              {meta.resolutionKind ? <code>{meta.resolutionKind}</code> : "—"}
            </span>
          </div>
          {meta.resolutionError && (
            <div className="inspector-row">
              <span className="inspector-label">Problem</span>
              <span className="inspector-value">{meta.resolutionError}</span>
            </div>
          )}
          <div className="inspector-row">
            <span className="inspector-label">PATH entries</span>
            <span className="inspector-value">{meta.pathEntriesChecked ?? 0} checked</span>
          </div>
        </details>
      </>
    );
  } else if (node.kind === "run") {
    const meta = runMeta(node);
    details = (
      <>
        <Row label="Status">{meta.status}</Row>
        <Row label="Planner">{meta.plannerAgent ?? "None"}</Row>
        <Row label="Tasks">{meta.counts.total}</Row>
        {meta.objective && <Row label="Objective">{meta.objective}</Row>}
      </>
    );
  } else if (node.kind === "task") {
    const meta = taskMeta(node);
    details = (
      <>
        <Row label="State">
          <span style={{ color: STATE_META[meta.state].color }}>{meta.state}</span>
        </Row>
        <Row label="Run">#{meta.runId}</Row>
        {meta.role && <Row label="Role">{meta.role}</Row>}
        <Row label="Depends on">
          {meta.dependencies.length ? meta.dependencies.map((id) => `#${id}`).join(", ") : "None"}
        </Row>
        {meta.worktreePath && (
          <Row label="Worktree">
            <code>{meta.worktreePath}</code>
          </Row>
        )}
        <Row label="Acceptance criteria">
          {meta.acceptanceCriteria.length ? (
            <ul className="inspector-criteria">
              {meta.acceptanceCriteria.map((criterion) => (
                <li key={criterion}>{criterion}</li>
              ))}
            </ul>
          ) : (
            "None"
          )}
        </Row>
        {meta.currentAttempt && (
          <>
            <Row label="Current attempt">
              #{meta.currentAttempt.attemptNumber} —{" "}
              {meta.currentAttempt.status.replaceAll("_", " ")}
            </Row>
            <Row label="Worker">{meta.currentAttempt.agent}</Row>
            {meta.currentAttempt.error && <Row label="Reason">{meta.currentAttempt.error}</Row>}
            {meta.currentAttempt.review && (
              <Row label="Review">{meta.currentAttempt.review.reason}</Row>
            )}
          </>
        )}
        {onRetry && ["failed", "blocked"].includes(meta.state) && (
          <button className="button" onClick={() => onRetry(meta.taskId)}>
            Retry task
          </button>
        )}
      </>
    );
  } else if (node.kind === "group") {
    details = <Row label="Effect">Visual organization only</Row>;
  } else if (node.kind === "note") {
    const text = "text" in node.meta ? String(node.meta.text) : "";
    details = (
      <>
        <Row label="Effect">Workspace metadata only</Row>
        {text && <Row label="Text">{text}</Row>}
      </>
    );
  } else {
    details = null;
  }

  const removable = node.kind === "group" || node.kind === "note";

  return (
    <Frame title={node.label} kind={node.kind} onClose={onClose}>
      {details}
      {targets.length > 0 && (
        <div className="inspector-connect">
          <label htmlFor="inspector-connection-target">Add supported connection</label>
          <div>
            <select
              id="inspector-connection-target"
              value={target}
              onChange={(event) => setTarget(event.target.value)}
            >
              <option value="">Choose target</option>
              {targets.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.label} ({candidate.kind})
                </option>
              ))}
            </select>
            <button
              className="button"
              disabled={!target}
              onClick={() => {
                onConnect(target);
                setTarget("");
              }}
            >
              Connect
            </button>
          </div>
        </div>
      )}
      {removable && (
        <button className="button inspector-delete" onClick={onDelete}>
          {`Delete ${node.kind}`}
        </button>
      )}
    </Frame>
  );
}
