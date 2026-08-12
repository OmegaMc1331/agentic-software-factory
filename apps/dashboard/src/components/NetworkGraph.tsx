import type { GraphEdge, GraphNode } from "../types";
import { agentMeta, runMeta, taskMeta } from "../types";
import { STATE_META } from "../state";
import { NET_NODE_HEIGHT, type NetworkLayout } from "../networkLayout";
import { truncate } from "../layout";

const EDGE_COLOR: Record<string, string> = {
  binds: "#59647a",
  uses: "#5b6b8c",
  contains: "#454e63",
  depends: "#5b7bb5",
};

const RUN_STATUS_COLOR: Record<string, string> = {
  planned: "#9ca3af",
  active: "#d97706",
  completed: "#16a34a",
  failed: "#dc2626",
};

function nodeColor(node: GraphNode): string {
  switch (node.kind) {
    case "agent":
      return agentMeta(node).available ? "#3d7dfd" : "#dc2626";
    case "role":
      return "#a78bfa";
    case "run":
      return "#38bdf8";
    case "task":
      return STATE_META[taskMeta(node).state].color;
  }
}

function isLive(node: GraphNode): boolean {
  if (node.kind === "run") return runMeta(node).status === "active";
  if (node.kind === "task") return taskMeta(node).state === "running";
  return false;
}

function edgeFlows(edge: GraphEdge, nodesById: Map<string, GraphNode>): boolean {
  const target = nodesById.get(edge.target);
  if (target?.kind === "task" && taskMeta(target).state === "running") return true;
  const source = nodesById.get(edge.source);
  if (source?.kind === "run" && runMeta(source).status === "active") return true;
  return false;
}

function rightGlyph(node: GraphNode): string | null {
  if (node.kind === "task") return `#${taskMeta(node).taskId}`;
  if (node.kind === "run") return `#${node.id.slice("run:".length)}`;
  return null;
}

export function NetworkGraph({
  layout,
  nodesById,
  activeId,
  neighborIds,
  onNodeEnter,
  onNodeLeave,
  onNodeClick,
  onBackgroundClick,
}: {
  layout: NetworkLayout;
  nodesById: Map<string, GraphNode>;
  activeId: string | null;
  neighborIds: string[];
  onNodeEnter: (id: string) => void;
  onNodeLeave: () => void;
  onNodeClick: (id: string) => void;
  onBackgroundClick: () => void;
}) {
  const isActive = (id: string) => activeId === id || neighborIds.includes(id);

  return (
    <svg
      className="network-svg"
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      width={layout.width}
      height={layout.height}
      role="img"
      aria-label="factory agent and task network"
      onClick={onBackgroundClick}
    >
      <defs>
        <marker
          id="net-arrow"
          viewBox="0 0 10 10"
          refX="8"
          refY="5"
          markerWidth="6"
          markerHeight="6"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke" />
        </marker>
      </defs>

      {layout.lanes.map((lane) => (
        <text key={lane.kind} className="network-lane-label" x={lane.x} y={16} textAnchor="middle">
          {lane.label}
        </text>
      ))}

      {layout.edges.map((edge) => {
        const connected = activeId === null || isActive(edge.source) || isActive(edge.target);
        const opacity = activeId === null ? 0.55 : connected ? 0.95 : 0.08;
        const flowing = activeId === null && edgeFlows(edge, nodesById);
        return (
          <path
            key={`${edge.source}->${edge.target}:${edge.kind}`}
            d={edge.path}
            fill="none"
            stroke={EDGE_COLOR[edge.kind]}
            strokeWidth={activeId !== null && connected ? 2 : 1.3}
            style={{ opacity }}
            className={flowing ? "net-edge-flow" : undefined}
            markerEnd="url(#net-arrow)"
          />
        );
      })}

      {layout.nodes.map((pos) => {
        const node = nodesById.get(pos.id);
        if (!node) return null;
        const color = nodeColor(node);
        const live = isLive(node);
        const dim = activeId !== null && !isActive(pos.id);
        const glyph = rightGlyph(node);
        const idX = node.kind === "run" ? pos.width - 26 : pos.width - 12;
        return (
          <g
            key={pos.id}
            className={`network-node${isActive(pos.id) ? " network-node--active" : ""}`}
            transform={`translate(${pos.x} ${pos.y})`}
            style={dim ? { opacity: 0.22 } : undefined}
            onMouseEnter={() => onNodeEnter(pos.id)}
            onMouseLeave={onNodeLeave}
            onClick={(event) => {
              event.stopPropagation();
              onNodeClick(pos.id);
            }}
          >
            {live && (
              <circle
                cx={pos.width / 2}
                cy={NET_NODE_HEIGHT / 2}
                r={NET_NODE_HEIGHT / 2 + 4}
                fill="none"
                stroke={color}
                strokeWidth={1.5}
                className="net-live-ring"
              />
            )}
            <rect
              x={0}
              y={0}
              width={pos.width}
              height={NET_NODE_HEIGHT}
              rx={7}
              fill="#161a22"
              stroke={color}
              strokeWidth={isActive(pos.id) ? 2 : 1.3}
            />
            <circle cx={13} cy={NET_NODE_HEIGHT / 2} r={4.5} fill={color} />
            <text x={25} y={NET_NODE_HEIGHT / 2 + 4} className="network-node-label">
              {truncate(node.label, 34)}
            </text>
            {glyph && (
              <text
                x={idX}
                y={NET_NODE_HEIGHT / 2 + 4}
                textAnchor="end"
                className="network-node-id"
              >
                {glyph}
              </text>
            )}
            {node.kind === "run" && (
              <circle
                cx={pos.width - 14}
                cy={NET_NODE_HEIGHT / 2}
                r={4}
                fill={RUN_STATUS_COLOR[runMeta(node).status] ?? "#9ca3af"}
              />
            )}
          </g>
        );
      })}
    </svg>
  );
}
