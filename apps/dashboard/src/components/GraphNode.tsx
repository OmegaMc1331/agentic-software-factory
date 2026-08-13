import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import type { AgentActivity, GraphNode as GraphNodeData } from "../types";
import { agentMeta, runMeta, taskMeta } from "../types";
import { STATE_META } from "../state";
import { truncate } from "../layout";

export interface FactoryNodeData extends Record<string, unknown> {
  node: GraphNodeData;
  activity: AgentActivity | null;
  dimmed: boolean;
}

export type FactoryFlowNode = Node<FactoryNodeData, "factory">;

function Handles({ kind }: { kind: GraphNodeData["kind"] }) {
  if (kind === "run") return null;
  return (
    <>
      <Handle type="target" position={Position.Left} id="target" className="graph-handle" />
      <Handle type="source" position={Position.Right} id="source" className="graph-handle" />
    </>
  );
}

export function GraphNode({ data, selected }: NodeProps<FactoryFlowNode>) {
  const { node, activity, dimmed } = data;
  const classes = [
    "graph-node",
    `graph-node--${node.kind}`,
    selected ? "graph-node--selected" : "",
    dimmed ? "graph-node--dimmed" : "",
  ]
    .filter(Boolean)
    .join(" ");

  if (node.kind === "agent") {
    const meta = agentMeta(node);
    const status = activity
      ? activity.taskId === null
        ? `Run #${activity.runId}`
        : `Task #${activity.taskId}`
      : meta.available
        ? "Available"
        : "Missing";
    return (
      <div className={classes} aria-label={`${node.label} agent, ${status}`}>
        <Handles kind={node.kind} />
        <span className="graph-agent-orbit" aria-hidden="true" />
        <span
          className={meta.available ? "graph-agent-status is-available" : "graph-agent-status"}
          aria-hidden="true"
        />
        <strong>{node.label}</strong>
        <span>{status}</span>
        <small>{meta.roles.length > 0 ? truncate(meta.roles.join(" / "), 24) : "No roles"}</small>
      </div>
    );
  }

  if (node.kind === "role") {
    return (
      <div className={classes} aria-label={`${node.label} role`}>
        <Handles kind={node.kind} />
        <span className="graph-node-kicker">Role</span>
        <strong>{node.label}</strong>
      </div>
    );
  }

  if (node.kind === "run") {
    const meta = runMeta(node);
    const status = meta.status;
    return (
      <div className={classes} aria-label={`${node.label} workflow, ${status}`}>
        <span className="graph-node-kicker">Workflow #{meta.runId}</span>
        <strong>{truncate(node.label, 34)}</strong>
        <span>{status}</span>
        <small>
          {meta.counts.completed} / {meta.counts.total} tasks
        </small>
      </div>
    );
  }

  if (node.kind === "task") {
    const meta = taskMeta(node);
    return (
      <div
        className={`${classes} graph-node--state-${meta.state}`}
        aria-label={`Task ${meta.taskId}, ${node.label}, ${meta.state}`}
        style={{ "--node-state": STATE_META[meta.state].color } as React.CSSProperties}
      >
        <Handles kind={node.kind} />
        <strong>#{meta.taskId}</strong>
        <span>{truncate(node.label, 24)}</span>
        <small>{meta.currentAttempt?.status === "reviewing" ? "review" : meta.state}</small>
      </div>
    );
  }

  if (node.kind === "group") {
    return (
      <div className={classes} aria-label={`${node.label} visual group`}>
        <Handles kind={node.kind} />
        <span className="graph-node-kicker">Group / visual only</span>
        <strong>{node.label}</strong>
      </div>
    );
  }

  const text = "text" in node.meta ? String(node.meta.text) : "";
  return (
    <div className={classes} aria-label={`${node.label} note`}>
      <Handles kind={node.kind} />
      <span className="graph-node-kicker">Note</span>
      <strong>{node.label}</strong>
      {text && <p>{truncate(text, 90)}</p>}
    </div>
  );
}
