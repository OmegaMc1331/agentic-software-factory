import { useEffect, useMemo, useState } from "react";
import { fetchRunArtifacts } from "../api";
import type { GraphEdge, GraphNode, RoleArtifact, RoleMeta } from "../types";
import {
  agentMeta,
  agentResolutionStatusLabel,
  operationStage,
  roleMeta,
  runMeta,
  STAGE_META,
  taskMeta,
} from "../types";
import { STATE_META } from "../state";
import { connectionKind } from "../graphWorkspace";
import { ArtifactList } from "./ArtifactList";
import { useRunArtifactsForTask } from "../artifactHelpers";

function executionClassFor(
  roleId: string | null,
  nodesById: Map<string, GraphNode>
): string | null {
  if (!roleId) return null;
  for (const node of nodesById.values()) {
    if (node.kind === "role") {
      const meta: RoleMeta = roleMeta(node);
      if (meta.id === roleId) return meta.executionClass;
    }
  }
  return null;
}

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
  const [runArtifacts, setRunArtifacts] = useState<RoleArtifact[]>([]);
  useEffect(() => {
    if (node?.kind !== "task") return;
    let active = true;
    setRunArtifacts([]);
    const runId = taskMeta(node).runId;
    fetchRunArtifacts(runId)
      .then((artifacts) => {
        if (active) setRunArtifacts(artifacts);
      })
      .catch(() => {
        if (active) setRunArtifacts([]);
      });
    return () => {
      active = false;
    };
  }, [node]);

  const selectedTaskId = node?.kind === "task" ? taskMeta(node).taskId : null;
  const selectedTaskDeps = node?.kind === "task" ? taskMeta(node).dependencies : [];
  const { produced, consumed } = useRunArtifactsForTask(
    runArtifacts,
    selectedTaskId,
    selectedTaskDeps
  );
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
    const stage = operationStage(meta.operation);
    const executionClass =
      meta.operation === null
        ? executionClassFor(meta.role, nodesById)
        : meta.operation === "implement" || meta.operation === "verify"
          ? "execution"
          : meta.operation;
    const attempt = meta.currentAttempt;
    details = (
      <>
        <Row label="State">
          <span style={{ color: STATE_META[meta.state].color }}>{meta.state}</span>
        </Row>
        <Row label="Run">#{meta.runId}</Row>
        {meta.operation && (
          <Row label="Operation">
            <span className="op-chip" style={{ color: STAGE_META[stage].color }}>
              {meta.operation}
            </span>
          </Row>
        )}
        {executionClass && <Row label="Execution class">{executionClass}</Row>}
        {meta.role ? <Row label="Role">{meta.role}</Row> : <Row label="Role">worker (default)</Row>}
        {attempt && <Row label="Agent">{attempt.agent}</Row>}
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
        {attempt && (
          <>
            <details className="agent-advanced">
              <summary>
                Attempt #{attempt.attemptNumber} — {attempt.status.replaceAll("_", " ")}
              </summary>
              <div className="inspector-row">
                <span className="inspector-label">Agent</span>
                <span className="inspector-value">{attempt.agent}</span>
              </div>
              {attempt.operation && (
                <div className="inspector-row">
                  <span className="inspector-label">Operation</span>
                  <span className="inspector-value">{attempt.operation}</span>
                </div>
              )}
              {attempt.error && (
                <div className="inspector-row">
                  <span className="inspector-label">Reason</span>
                  <span className="inspector-value">{attempt.error}</span>
                </div>
              )}
              {attempt.review && (
                <div className="inspector-row">
                  <span className="inspector-label">Review</span>
                  <span className="inspector-value">{attempt.review.reason}</span>
                </div>
              )}
              {attempt.evidence && (
                <div className="inspector-row">
                  <span className="inspector-label">Changed files</span>
                  <span className="inspector-value">
                    {attempt.evidence.changedFiles.length
                      ? attempt.evidence.changedFiles.join(", ")
                      : "none (no repository changes required)"}
                  </span>
                </div>
              )}
              {attempt.evidence && attempt.evidence.commands.length > 0 && (
                <div className="inspector-row">
                  <span className="inspector-label">Commands</span>
                  <span className="inspector-value">{attempt.evidence.commands.join("; ")}</span>
                </div>
              )}
            </details>
            {attempt.evidence && (
              <div className="inspector-row">
                <span className="inspector-label">Diff</span>
                <span className="inspector-value">
                  {attempt.evidence.diffPatch ? "captured" : attempt.evidence.diffSummary || "—"}
                </span>
              </div>
            )}
          </>
        )}
        <div className="inspector-artifacts">
          <span className="inspector-label">Produced artifacts</span>
          <ArtifactList artifacts={produced} empty="None." />
          <span className="inspector-label">Consumed artifacts (dependencies)</span>
          <ArtifactList artifacts={consumed} empty="None." />
        </div>
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
