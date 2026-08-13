import {
  ConnectionLineType,
  MarkerType,
  ReactFlow,
  ReactFlowProvider,
  getNodesBounds,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Connection,
  type NodeMouseHandler,
  type OnConnect,
  type ReactFlowInstance,
  type XYPosition,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useMemo, useRef } from "react";
import type {
  AgentActivity,
  GraphEdge as GraphEdgeData,
  GraphNode as GraphNodeData,
} from "../types";
import { runMeta, taskMeta } from "../types";
import type { NetworkLayout } from "../networkLayout";
import { GraphEdge, type FactoryFlowEdge } from "./GraphEdge";
import { GraphNode, type FactoryFlowNode } from "./GraphNode";

export interface AgentGraphHandle {
  fit: () => void;
  center: () => void;
  centerOn: (id: string) => void;
  zoomIn: () => void;
  zoomOut: () => void;
  viewportCenter: () => XYPosition;
}

const nodeTypes = { factory: GraphNode };
const edgeTypes = { factory: GraphEdge };

interface AgentGraphProps {
  layout: NetworkLayout;
  nodesById: Map<string, GraphNodeData>;
  edges: GraphEdgeData[];
  positions: Record<string, XYPosition>;
  positionRevision: number;
  selectedNodeId: string | null;
  selectedEdgeId: string | null;
  neighborIds: string[];
  activities: Map<string, AgentActivity | null>;
  onNodeSelect: (id: string) => void;
  onEdgeSelect: (id: string) => void;
  onBackgroundClick: () => void;
  onNodeDragStop: (id: string, position: XYPosition) => void;
  onConnect: OnConnect;
  isValidConnection: (connection: Connection | FactoryFlowEdge) => boolean;
  onZoomChange: (zoom: number) => void;
}

function GraphSurface(
  {
    layout,
    nodesById,
    edges: graphEdges,
    positions,
    positionRevision,
    selectedNodeId,
    selectedEdgeId,
    neighborIds,
    activities,
    onNodeSelect,
    onEdgeSelect,
    onBackgroundClick,
    onNodeDragStop,
    onConnect,
    isValidConnection,
    onZoomChange,
  }: AgentGraphProps,
  ref: React.ForwardedRef<AgentGraphHandle>
) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const instanceRef = useRef<ReactFlowInstance<FactoryFlowNode, FactoryFlowEdge> | null>(null);
  const positionRevisionRef = useRef(positionRevision);
  const { fitView, setCenter, zoomIn, zoomOut, screenToFlowPosition, getNode, getNodes } =
    useReactFlow<FactoryFlowNode, FactoryFlowEdge>();
  const neighborSet = useMemo(() => new Set(neighborIds), [neighborIds]);

  const buildNodes = useCallback(
    (): FactoryFlowNode[] =>
      layout.nodes.flatMap((position) => {
        const node = nodesById.get(position.id);
        if (!node) return [];
        return [
          {
            id: node.id,
            type: "factory",
            position: positions[node.id] ?? { x: position.x, y: position.y },
            selected: node.id === selectedNodeId,
            draggable: true,
            selectable: true,
            deletable: false,
            zIndex: node.kind === "group" ? -1 : node.id === selectedNodeId ? 4 : 1,
            style:
              node.kind === "group"
                ? { width: position.width, height: position.height }
                : undefined,
            data: {
              node,
              activity: node.kind === "agent" ? (activities.get(node.id) ?? null) : null,
              dimmed:
                selectedNodeId !== null && node.id !== selectedNodeId && !neighborSet.has(node.id),
            },
          },
        ];
      }),
    [activities, layout.nodes, neighborSet, nodesById, positions, selectedNodeId]
  );

  const buildEdges = useCallback(
    (): FactoryFlowEdge[] =>
      graphEdges.map((edge) => {
        const target = nodesById.get(edge.target);
        const source = nodesById.get(edge.source);
        const active =
          (edge.kind === "plans" &&
            source?.kind === "run" &&
            runMeta(source).status === "planning") ||
          ((edge.kind === "works" || edge.kind === "reviews") &&
            target?.kind === "task" &&
            taskMeta(target).state === "running");
        const connected =
          selectedNodeId === null ||
          edge.source === selectedNodeId ||
          edge.target === selectedNodeId;
        const directional = !["custom", "membership", "binds"].includes(edge.kind);
        return {
          id: edge.id,
          source: edge.source,
          target: edge.target,
          type: "factory",
          selected: edge.id === selectedEdgeId,
          selectable: true,
          deletable: false,
          animated: active,
          markerEnd: directional ? { type: MarkerType.ArrowClosed } : undefined,
          data: {
            kind: edge.kind,
            editable: edge.editable,
            semantic: edge.semantic,
            dimmed: !connected,
            active,
          },
        };
      }),
    [graphEdges, nodesById, selectedEdgeId, selectedNodeId]
  );

  const [nodes, setNodes, onNodesChange] = useNodesState<FactoryFlowNode>(buildNodes());
  const [edges, setEdges, onEdgesChange] = useEdgesState<FactoryFlowEdge>(buildEdges());

  useEffect(() => {
    setNodes((current) => {
      const currentById = new Map(current.map((node) => [node.id, node]));
      const shouldReplacePositions = positionRevisionRef.current !== positionRevision;
      positionRevisionRef.current = positionRevision;
      return buildNodes().map((node) => ({
        ...node,
        position:
          shouldReplacePositions || !currentById.has(node.id)
            ? node.position
            : currentById.get(node.id)!.position,
      }));
    });
  }, [buildNodes, positionRevision, setNodes]);

  useEffect(() => setEdges(buildEdges()), [buildEdges, setEdges]);

  const fit = useCallback(() => {
    void fitView({ padding: 0.1, minZoom: 0.12, maxZoom: 1.8, duration: 320 });
  }, [fitView]);

  const centerOn = useCallback(
    (id: string) => {
      const node = getNode(id);
      if (!node) return;
      const width = node.measured?.width ?? node.width ?? 140;
      const height = node.measured?.height ?? node.height ?? 70;
      void setCenter(node.position.x + width / 2, node.position.y + height / 2, {
        zoom: instanceRef.current?.getZoom() ?? 1,
        duration: 260,
      });
    },
    [getNode, setCenter]
  );

  useImperativeHandle(
    ref,
    () => ({
      fit,
      center() {
        if (selectedNodeId) {
          centerOn(selectedNodeId);
          return;
        }
        const bounds = getNodesBounds(getNodes());
        void setCenter(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2, {
          zoom: instanceRef.current?.getZoom() ?? 1,
          duration: 260,
        });
      },
      centerOn,
      zoomIn() {
        void zoomIn({ duration: 180 });
      },
      zoomOut() {
        void zoomOut({ duration: 180 });
      },
      viewportCenter() {
        const rect = wrapperRef.current?.getBoundingClientRect();
        if (!rect) return { x: 0, y: 0 };
        return screenToFlowPosition({
          x: rect.left + rect.width / 2,
          y: rect.top + rect.height / 2,
        });
      },
    }),
    [centerOn, fit, getNodes, screenToFlowPosition, selectedNodeId, setCenter, zoomIn, zoomOut]
  );

  const selectNode: NodeMouseHandler<FactoryFlowNode> = (_event, node) => onNodeSelect(node.id);

  return (
    <div className="agent-canvas" ref={wrapperRef} aria-label="Factory agent and task graph">
      <ReactFlow<FactoryFlowNode, FactoryFlowEdge>
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={selectNode}
        onEdgeClick={(_event, edge) => onEdgeSelect(edge.id)}
        onPaneClick={onBackgroundClick}
        onNodeDragStop={(_event, node) => onNodeDragStop(node.id, node.position)}
        onConnect={onConnect}
        isValidConnection={isValidConnection}
        onInit={(instance) => {
          instanceRef.current = instance;
          window.requestAnimationFrame(fit);
        }}
        onMove={(_event, viewport) => onZoomChange(viewport.zoom)}
        minZoom={0.12}
        maxZoom={3.2}
        fitView
        fitViewOptions={{ padding: 0.1, minZoom: 0.12, maxZoom: 1.8 }}
        panOnDrag
        zoomOnScroll
        zoomOnPinch
        zoomOnDoubleClick={false}
        connectionLineType={ConnectionLineType.Bezier}
        deleteKeyCode={null}
        selectionKeyCode={null}
        multiSelectionKeyCode={null}
        proOptions={{ hideAttribution: true }}
      />
    </div>
  );
}

const ForwardedGraphSurface = forwardRef(GraphSurface);

export const AgentGraph = forwardRef<AgentGraphHandle, AgentGraphProps>(
  function AgentGraph(props, ref) {
    return (
      <ReactFlowProvider>
        <ForwardedGraphSurface {...props} ref={ref} />
      </ReactFlowProvider>
    );
  }
);
