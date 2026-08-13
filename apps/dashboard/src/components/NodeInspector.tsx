import { useEffect, useMemo, useState } from "react";
import type { GraphEdge, GraphNode } from "../types";
import { roleMeta, runMeta, taskMeta } from "../types";
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
}: {
  node: GraphNode | null;
  edge: GraphEdge | null;
  nodesById: Map<string, GraphNode>;
  onClose: () => void;
  onDelete: () => void;
  onConnect: (targetId: string) => void;
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
  if (node.kind === "role") {
    details = <Row label="Assigned agent">{roleMeta(node).agent}</Row>;
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
        <Row label="Depends on">
          {meta.dependencies.length ? meta.dependencies.map((id) => `#${id}`).join(", ") : "None"}
        </Row>
        {meta.worktreePath && (
          <Row label="Worktree">
            <code>{meta.worktreePath}</code>
          </Row>
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

  const removable = node.kind === "role" || node.kind === "group" || node.kind === "note";

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
          {node.kind === "role" ? "Unassign role" : `Delete ${node.kind}`}
        </button>
      )}
    </Frame>
  );
}
