import type { GraphEdgeKind } from "../types";

const EDGE_COLOR: Record<GraphEdgeKind, string> = {
  binds: "#495270",
  uses: "#55638a",
  contains: "#3a4459",
  depends: "#4d5f8f",
};
const EDGE_FLOW = "#8fa8d9";
const EDGE_ALERT = "#c98d4b";
const EDGE_FAIL = "#d4635c";

export function GraphEdge({
  path,
  kind,
  tone,
  flowing,
  emphasized,
  dimmed,
}: {
  path: string;
  kind: GraphEdgeKind;
  tone: "ok" | "blocked" | "failed";
  flowing: boolean;
  emphasized: boolean;
  dimmed: boolean;
}) {
  const stroke =
    tone === "failed"
      ? EDGE_FAIL
      : tone === "blocked"
        ? EDGE_ALERT
        : flowing
          ? EDGE_FLOW
          : EDGE_COLOR[kind];
  const opacity = dimmed ? 0.06 : emphasized || flowing ? 0.95 : 0.5;
  const strokeWidth = emphasized || flowing ? 1.7 : 1.1;
  return (
    <path
      d={path}
      fill="none"
      stroke={stroke}
      strokeWidth={strokeWidth}
      strokeDasharray={tone === "blocked" ? "4 4" : undefined}
      style={{ opacity }}
      className={flowing && !dimmed ? "net-edge-flow" : undefined}
      markerEnd="url(#net-arrow)"
    />
  );
}
