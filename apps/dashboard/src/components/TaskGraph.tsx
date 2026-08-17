import { useMemo } from "react";
import type { Task } from "../types";
import { operationStage, STAGE_META } from "../types";
import { STATE_META } from "../state";
import { NODE_HEIGHT, NODE_WIDTH, computeLayout, truncate } from "../layout";

const BEND = 18;

function edgePath(sourceX: number, sourceY: number, targetX: number, targetY: number): string {
  const midY = (sourceY + targetY) / 2;
  return `M ${targetX} ${targetY} C ${targetX} ${midY + BEND}, ${sourceX} ${
    midY - BEND
  }, ${sourceX} ${sourceY}`;
}

export function TaskGraph({ tasks }: { tasks: Task[] }) {
  const layout = useMemo(() => computeLayout(tasks), [tasks]);
  const byId = new Map(tasks.map((t) => [t.id, t]));

  return (
    <svg
      className="graph"
      width={layout.width}
      height={layout.height}
      role="img"
      aria-label="task dependency graph"
    >
      <defs>
        <marker
          id="arrow"
          viewBox="0 0 10 10"
          refX="9"
          refY="5"
          markerWidth="7"
          markerHeight="7"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 10 5 L 0 10 z" fill="#d1d5db" />
        </marker>
      </defs>
      {layout.edges.map((edge, index) => {
        const source = layout.nodes.find((n) => n.id === edge.source);
        const target = layout.nodes.find((n) => n.id === edge.target);
        if (!source || !target) return null;
        return (
          <path
            key={index}
            d={edgePath(
              source.x + NODE_WIDTH / 2,
              source.y + NODE_HEIGHT,
              target.x + NODE_WIDTH / 2,
              target.y
            )}
            fill="none"
            stroke="#d1d5db"
            strokeWidth={1.5}
            markerEnd="url(#arrow)"
          />
        );
      })}
      {layout.nodes.map((node) => {
        const task = byId.get(node.id);
        if (!task) return null;
        const meta = STATE_META[task.state];
        const stage = task.operation ? operationStage(task.operation) : null;
        return (
          <g key={node.id} className="graph-node">
            <rect
              x={node.x}
              y={node.y}
              width={NODE_WIDTH}
              height={NODE_HEIGHT}
              rx={6}
              fill="#ffffff"
              stroke={meta.color}
              strokeWidth={1.5}
            />
            <circle cx={node.x + 14} cy={node.y + NODE_HEIGHT / 2} r={5} fill={meta.color} />
            <text x={node.x + 28} y={node.y + NODE_HEIGHT / 2 + 4} className="graph-label">
              {truncate(task.title)}
            </text>
            {stage && (
              <text
                x={node.x + NODE_WIDTH - 12}
                y={node.y + NODE_HEIGHT - 8}
                className="graph-op-tag"
                textAnchor="end"
                fill={STAGE_META[stage].color}
              >
                {STAGE_META[stage].short}
              </text>
            )}
            <text
              x={node.x + NODE_WIDTH - 12}
              y={node.y + NODE_HEIGHT / 2 + 4}
              className="graph-id"
              textAnchor="end"
            >
              #{node.id}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
