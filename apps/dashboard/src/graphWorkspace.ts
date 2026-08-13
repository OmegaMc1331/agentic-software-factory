import type { Connection, Node } from "@xyflow/react";
import type {
  GraphData,
  GraphEdge,
  GraphNode,
  GraphPosition,
  GraphWorkspace,
  WorkspaceEdge,
} from "./types";

export function mergeGraphWorkspace(data: GraphData, workspace: GraphWorkspace): GraphData {
  const customNodes: GraphNode[] = workspace.customNodes.map((node) =>
    node.kind === "group"
      ? {
          id: node.id,
          kind: "group",
          label: node.label,
          meta: { visualOnly: true },
        }
      : {
          id: node.id,
          kind: "note",
          label: node.label,
          meta: { text: node.text ?? "", visualOnly: true },
        }
  );
  const customEdges: GraphEdge[] = workspace.edges.map((edge) => ({
    ...edge,
    editable: true,
    semantic: "visual",
  }));
  return {
    ...data,
    nodes: [...data.nodes, ...customNodes],
    edges: [...data.edges, ...customEdges],
  };
}

export function workspacePosition(
  id: string,
  automatic: GraphPosition,
  workspace: GraphWorkspace
): GraphPosition {
  return workspace.nodes[id] ?? automatic;
}

export function positionsFromNodes(nodes: Node[]): Record<string, GraphPosition> {
  return Object.fromEntries(
    nodes.map((node) => [
      node.id,
      { x: Math.round(node.position.x), y: Math.round(node.position.y) },
    ])
  );
}

export function connectionKind(
  source: GraphNode | undefined,
  target: GraphNode | undefined
): WorkspaceEdge["kind"] | "assignment" | null {
  if (!source || !target || source.id === target.id) return null;
  if (source.kind === "role" && target.kind === "agent") return "assignment";
  if (source.kind === "agent" && target.kind === "agent") return "custom";
  if (
    (source.kind === "group" && target.kind !== "group") ||
    (target.kind === "group" && source.kind !== "group")
  ) {
    return "membership";
  }
  return null;
}

export function validateConnection(
  connection: Connection,
  nodesById: Map<string, GraphNode>,
  edges: GraphEdge[]
): string | null {
  if (!connection.source || !connection.target) return "Choose a source and target node.";
  const source = nodesById.get(connection.source);
  const target = nodesById.get(connection.target);
  const kind = connectionKind(source, target);
  if (kind === null) return "These node types do not support a configurable connection.";
  const edgeKind = kind === "assignment" ? "binds" : kind;
  if (
    edges.some(
      (edge) =>
        edge.source === connection.source &&
        edge.target === connection.target &&
        edge.kind === edgeKind
    )
  ) {
    return "This connection already exists.";
  }
  return null;
}

export function findFreePosition(
  center: GraphPosition,
  occupied: Array<{ x: number; y: number; width?: number; height?: number }>
): GraphPosition {
  const candidates = [{ x: center.x - 70, y: center.y - 35 }];
  for (let ring = 1; ring <= 5; ring += 1) {
    const radius = ring * 88;
    for (let step = 0; step < 12; step += 1) {
      const angle = (step / 12) * Math.PI * 2;
      candidates.push({
        x: center.x + Math.cos(angle) * radius - 70,
        y: center.y + Math.sin(angle) * radius - 35,
      });
    }
  }
  return (
    candidates.find(
      (candidate) =>
        !occupied.some((node) => {
          const width = node.width ?? 150;
          const height = node.height ?? 80;
          return (
            candidate.x < node.x + width + 22 &&
            candidate.x + 150 + 22 > node.x &&
            candidate.y < node.y + height + 22 &&
            candidate.y + 80 + 22 > node.y
          );
        })
    ) ?? candidates[candidates.length - 1]
  );
}

export function nextWorkspaceId(kind: "group" | "note", label: string): string {
  const slug =
    label
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || kind;
  return `${kind}:${slug}-${Date.now().toString(36)}`;
}
