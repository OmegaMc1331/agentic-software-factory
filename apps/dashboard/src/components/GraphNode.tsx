import { Handle, Position, type Node, type NodeProps } from "@xyflow/react";
import type { AgentActivity, GraphNode as GraphNodeData } from "../types";
import {
  agentMeta,
  githubIssueMeta,
  githubPrMeta,
  operationStage,
  roleMeta,
  runMeta,
  STAGE_META,
  taskMeta,
} from "../types";
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
        : meta.status === "broken"
          ? "Broken installation"
          : "Missing";
    return (
      <div className={classes} aria-label={`${node.label} agent, ${status}`}>
        <Handles kind={node.kind} />
        <span className="graph-agent-orbit" aria-hidden="true" />
        <span
          className={
            meta.available
              ? "graph-agent-status is-available"
              : meta.status === "broken"
                ? "graph-agent-status is-broken"
                : "graph-agent-status"
          }
          aria-hidden="true"
        />
        <strong>{node.label}</strong>
        <span>{status}</span>
        <small>{meta.roles.length > 0 ? truncate(meta.roles.join(" / "), 24) : "No roles"}</small>
      </div>
    );
  }

  if (node.kind === "role") {
    const meta = roleMeta(node);
    return (
      <div className={classes} aria-label={`${node.label} role`}>
        <Handles kind={node.kind} />
        <span className="graph-node-kicker">
          {meta.kind === "core" ? "Core role" : "Custom role"}
        </span>
        <strong>{truncate(node.label, 18)}</strong>
        {meta.assignments.length > 1 && <small>{meta.assignments.length} agents</small>}
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
    const showRole = meta.role !== null && meta.role !== "worker";
    const stage = meta.operation ? operationStage(meta.operation) : null;
    return (
      <div
        className={`${classes} graph-node--state-${meta.state}`}
        aria-label={`Task ${meta.taskId}, ${node.label}, ${meta.state}`}
        style={{ "--node-state": STATE_META[meta.state].color } as React.CSSProperties}
      >
        <Handles kind={node.kind} />
        <strong>#{meta.taskId}</strong>
        <span>{truncate(node.label, 24)}</span>
        <small>
          {meta.currentAttempt?.status === "reviewing" ? "review" : meta.state}
          {showRole ? ` · ${meta.role}` : ""}
          {stage && (
            <span
              className="graph-node-op"
              style={{ color: STAGE_META[stage].color }}
              aria-label={`operation ${meta.operation}`}
            >
              {" "}
              ◦ {STAGE_META[stage].short}
            </span>
          )}
        </small>
      </div>
    );
  }

  if (node.kind === "github_issue") {
    const meta = githubIssueMeta(node);
    return (
      <div className={classes} aria-label={`GitHub issue ${meta.number}, ${meta.title}`}>
        <Handles kind={node.kind} />
        <span className="graph-node-kicker">GitHub Issue</span>
        <strong>#{meta.number}</strong>
        <span>{truncate(meta.title, 26)}</span>
        <small>{meta.state === "closed" ? "closed" : "open"}</small>
      </div>
    );
  }

  if (node.kind === "github_pr") {
    const meta = githubPrMeta(node);
    return (
      <div className={classes} aria-label={`Pull request ${meta.number}, ${meta.state}`}>
        <Handles kind={node.kind} />
        <span className="graph-node-kicker">Pull Request</span>
        <strong>#{meta.number}</strong>
        <span>{meta.state}</span>
        {meta.isDraft && <small>draft</small>}
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
