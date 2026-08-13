import { BaseEdge, getBezierPath, type Edge, type EdgeProps } from "@xyflow/react";
import type { GraphEdgeKind } from "../types";

export interface FactoryEdgeData extends Record<string, unknown> {
  kind: GraphEdgeKind;
  editable: boolean;
  semantic: string;
  dimmed: boolean;
  active: boolean;
}

export type FactoryFlowEdge = Edge<FactoryEdgeData, "factory">;

export function GraphEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  markerEnd,
  selected,
  data,
}: EdgeProps<FactoryFlowEdge>) {
  const [path] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
    curvature: 0.32,
  });
  const kind = data?.kind ?? "custom";
  const className = [
    "graph-edge",
    `graph-edge--${kind}`,
    selected ? "graph-edge--selected" : "",
    data?.dimmed ? "graph-edge--dimmed" : "",
    data?.active ? "graph-edge--active" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return <BaseEdge id={id} path={path} markerEnd={markerEnd} className={className} />;
}
