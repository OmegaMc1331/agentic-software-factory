import type { NetworkNodePos } from "../networkLayout";
import { STATE_META } from "../state";
import type { AgentActivity, GraphNode as GraphNodeData } from "../types";
import { agentMeta, runMeta, taskMeta } from "../types";
import { truncate } from "../layout";

const AGENT_COLOR = "#3d7dfd";
const AGENT_MISSING = "#e5534b";
const AGENT_AVAILABLE = "#2ea467";
const ROLE_COLOR = "#a78bfa";
const RUN_STATUS_COLOR: Record<string, string> = {
  planned: "#9ca3af",
  active: "#38bdf8",
  completed: "#2ea467",
  failed: "#e5534b",
};

export function GraphNode({
  pos,
  node,
  selected,
  dimmed,
  activity,
  onEnter,
  onLeave,
}: {
  pos: NetworkNodePos;
  node: GraphNodeData;
  selected: boolean;
  dimmed: boolean;
  activity: AgentActivity | null;
  onEnter: (id: string) => void;
  onLeave: () => void;
}) {
  const className = [
    "graph-node",
    `graph-node--${pos.kind}`,
    selected ? "graph-node--selected" : "",
    dimmed ? "graph-node--dimmed" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const common = {
    className,
    transform: `translate(${pos.x} ${pos.y})`,
    style: dimmed ? { opacity: 0.22 } : undefined,
    onMouseEnter: () => onEnter(pos.id),
    onMouseLeave: onLeave,
  };

  if (pos.kind === "agent") {
    const meta = agentMeta(node);
    const color = meta.available ? AGENT_COLOR : AGENT_MISSING;
    const roleCaption = meta.roles.length === 0 ? "no roles" : truncate(meta.roles.join(" · "), 20);
    const statusText = activity
      ? activity.taskId !== null
        ? `working · #${activity.taskId}`
        : `orchestrating run #${activity.runId}`
      : null;
    return (
      <g {...common}>
        <title>{`${node.label} agent`}</title>
        {activity && <circle cx={65} cy={36} r={34} className="net-pulse net-pulse--agent" />}
        <circle
          cx={65}
          cy={36}
          r={30}
          fill="#161a22"
          stroke={color}
          strokeWidth={selected ? 2.5 : 1.4}
        />
        <circle
          cx={86}
          cy={15}
          r={4.5}
          fill={meta.available ? AGENT_AVAILABLE : AGENT_MISSING}
          className={meta.available ? "" : "agent-dot-hollow"}
        />
        <text x={65} y={70} textAnchor="middle" className="gn-label gn-label--agent">
          {node.label}
        </text>
        <text x={65} y={86} textAnchor="middle" className="gn-sub">
          {roleCaption}
        </text>
        {statusText && (
          <text x={65} y={100} textAnchor="middle" className="gn-activity">
            {statusText}
          </text>
        )}
      </g>
    );
  }

  if (pos.kind === "role") {
    return (
      <g {...common}>
        <title>{`${node.label} role`}</title>
        <rect
          x={0}
          y={0}
          width={pos.width}
          height={pos.height}
          rx={12}
          fill="#161a22"
          stroke={ROLE_COLOR}
          strokeOpacity={selected ? 1 : 0.55}
          strokeWidth={selected ? 2 : 1}
        />
        <text x={pos.width / 2} y={16} textAnchor="middle" className="gn-label gn-label--role">
          {node.label}
        </text>
      </g>
    );
  }

  if (pos.kind === "run") {
    const status = runMeta(node).status;
    const statusColor = RUN_STATUS_COLOR[status] ?? "#9ca3af";
    const active = status === "active";
    return (
      <g {...common}>
        <title>{`${node.label} run, ${status}`}</title>
        {active && (
          <rect
            x={-4}
            y={-4}
            width={pos.width + 8}
            height={pos.height + 8}
            rx={16}
            className="net-pulse"
          />
        )}
        <rect
          x={0}
          y={0}
          width={pos.width}
          height={pos.height}
          rx={13}
          fill="#161a22"
          stroke={statusColor}
          strokeOpacity={selected ? 1 : 0.6}
          strokeWidth={selected ? 2 : 1.1}
        />
        <circle cx={13} cy={pos.height / 2} r={4} fill={statusColor} />
        <text x={24} y={17} className="gn-label gn-label--run">
          {node.label}
        </text>
      </g>
    );
  }

  const meta = taskMeta(node);
  const stateMeta = STATE_META[meta.state];
  const idText = `#${meta.taskId} `;
  const budget = Math.max(3, Math.floor((pos.width - 30 - idText.length * 6.4) / 6.2));
  const running = meta.state === "running";
  return (
    <g {...common}>
      <title>{`${node.label} — ${meta.state}`}</title>
      {running && (
        <rect
          x={-3}
          y={-3}
          width={pos.width + 6}
          height={pos.height + 6}
          rx={14}
          className="net-pulse"
        />
      )}
      <rect
        x={0}
        y={0}
        width={pos.width}
        height={pos.height}
        rx={11}
        fill={running ? "rgba(217,119,6,0.07)" : "#161a22"}
        stroke={stateMeta.color}
        strokeWidth={selected ? 2 : running || meta.state === "failed" ? 1.5 : 1}
        strokeDasharray={meta.state === "blocked" ? "4 3" : undefined}
      />
      <circle cx={12.5} cy={pos.height / 2} r={3.5} fill={stateMeta.color} />
      <text x={22} y={15} className="gn-label gn-label--task">
        <tspan className="gn-task-id" fill={stateMeta.color}>
          {idText}
        </tspan>
        <tspan className="gn-task-title">{truncate(node.label, budget)}</tspan>
      </text>
    </g>
  );
}
