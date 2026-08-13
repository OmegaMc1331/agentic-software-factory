import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Connection, XYPosition } from "@xyflow/react";
import {
  fetchConfig,
  fetchGraph,
  fetchGraphWorkspace,
  saveConfig,
  saveGraphWorkspace,
} from "../api";
import {
  connectionKind,
  findFreePosition,
  mergeGraphWorkspace,
  nextWorkspaceId,
  validateConnection,
} from "../graphWorkspace";
import type {
  AgentEntry,
  ConfigData,
  GraphData,
  GraphEdge,
  GraphNode,
  GraphWorkspace,
  TaskState,
} from "../types";
import { agentActivity, agentMeta, taskMeta, type AgentActivity } from "../types";
import { computeNetworkLayout, neighborsOf, type NetworkLayout } from "../networkLayout";
import { STATE_META } from "../state";
import { AddNodeMenu } from "./AddNodeMenu";
import { AgentConsole } from "./AgentConsole";
import { AgentGraph, type AgentGraphHandle } from "./AgentGraph";
import { GraphToolbar, type RunOption } from "./GraphToolbar";
import { NodeInspector } from "./NodeInspector";

const POLL_MS = 3000;
const STATE_ORDER: TaskState[] = ["pending", "ready", "running", "blocked", "failed", "completed"];

function Loading() {
  return <p className="empty-title">Loading...</p>;
}

function LoadError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="empty net-empty" role="alert">
      <p className="empty-title">Could not load the Agent Graph.</p>
      <p className="error">{message}</p>
      <div className="empty-actions">
        <button className="button" onClick={onRetry}>
          Retry
        </button>
      </div>
    </div>
  );
}

function layoutKey(nodes: GraphNode[], edges: GraphEdge[]): string {
  return [
    ...nodes.map((node) => `${node.id}:${node.kind}`).sort(),
    ...edges.map((edge) => edge.id).sort(),
  ].join("|");
}

export function NetworkView() {
  const [data, setData] = useState<GraphData | null>(null);
  const [workspace, setWorkspace] = useState<GraphWorkspace | null>(null);
  const [config, setConfig] = useState<ConfigData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [live, setLive] = useState(true);
  const [runFilter, setRunFilter] = useState<number | null>(null);
  const [showTasks, setShowTasks] = useState<boolean | null>(null);
  const [showDependencies, setShowDependencies] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [positionRevision, setPositionRevision] = useState(0);
  const graphRef = useRef<AgentGraphHandle>(null);
  const layoutCache = useRef<{ key: string; layout: NetworkLayout } | null>(null);

  const reload = useCallback(() => {
    Promise.all([fetchGraph(), fetchGraphWorkspace(), fetchConfig()])
      .then(([nextData, nextWorkspace, nextConfig]) => {
        setData(nextData);
        setWorkspace(nextWorkspace);
        setConfig(nextConfig);
        setError(null);
        if (nextWorkspace.warning) setOperationError(nextWorkspace.warning);
      })
      .catch((reason: Error) => setError(reason.message));
  }, []);

  const reloadGraph = useCallback(() => {
    fetchGraph()
      .then((next) => {
        setData(next);
        setPollError(null);
      })
      .catch((reason: Error) => setPollError(reason.message));
  }, []);

  useEffect(() => reload(), [reload]);
  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(reloadGraph, POLL_MS);
    return () => window.clearInterval(timer);
  }, [live, reloadGraph]);

  const merged = useMemo(
    () => (data && workspace ? mergeGraphWorkspace(data, workspace) : null),
    [data, workspace]
  );
  const allNodesById = useMemo(
    () => new Map((merged?.nodes ?? []).map((node) => [node.id, node])),
    [merged]
  );
  const activities = useMemo(() => {
    const result = new Map<string, AgentActivity | null>();
    if (!merged) return result;
    for (const node of merged.nodes) {
      if (node.kind === "agent") {
        result.set(node.id, agentActivity(node.id, allNodesById, merged.edges));
      }
    }
    return result;
  }, [allNodesById, merged]);

  const runOptions: RunOption[] = useMemo(
    () =>
      (merged?.nodes ?? [])
        .filter((node) => node.kind === "run")
        .map((node) => ({ id: Number(node.id.slice(4)), label: node.label })),
    [merged]
  );
  const effectiveShowTasks =
    runFilter !== null ? true : (showTasks ?? (merged?.metadata.tasks ?? 0) > 0);
  const filteredNodes = useMemo(
    () =>
      (merged?.nodes ?? []).filter((node) => {
        if (node.kind === "task") {
          return effectiveShowTasks && (runFilter === null || taskMeta(node).runId === runFilter);
        }
        if (node.kind === "run") {
          return runFilter === null || Number(node.id.slice(4)) === runFilter;
        }
        return true;
      }),
    [effectiveShowTasks, merged, runFilter]
  );
  const visibleIds = useMemo(() => new Set(filteredNodes.map((node) => node.id)), [filteredNodes]);
  const filteredEdges = useMemo(
    () =>
      (merged?.edges ?? []).filter(
        (edge) =>
          (edge.kind !== "depends" || showDependencies) &&
          visibleIds.has(edge.source) &&
          visibleIds.has(edge.target)
      ),
    [merged, showDependencies, visibleIds]
  );
  const nodesById = useMemo(
    () => new Map(filteredNodes.map((node) => [node.id, node])),
    [filteredNodes]
  );

  const key = layoutKey(filteredNodes, filteredEdges);
  if (!layoutCache.current || layoutCache.current.key !== key) {
    layoutCache.current = {
      key,
      layout: computeNetworkLayout(filteredNodes, filteredEdges),
    };
  }
  const layout = layoutCache.current.layout;
  const neighbors = useMemo(
    () => (selectedNodeId ? neighborsOf(layout.nodes, layout.edges, selectedNodeId) : []),
    [layout, selectedNodeId]
  );
  const selectedNode = selectedNodeId ? (allNodesById.get(selectedNodeId) ?? null) : null;
  const selectedEdge = selectedEdgeId
    ? (merged?.edges.find((edge) => edge.id === selectedEdgeId) ?? null)
    : null;

  const persistWorkspace = useCallback(async (next: GraphWorkspace): Promise<boolean> => {
    setWorkspace(next);
    setOperationError(null);
    try {
      await saveGraphWorkspace(next);
      return true;
    } catch (reason) {
      setOperationError((reason as Error).message);
      return false;
    }
  }, []);

  const freePosition = useCallback((): XYPosition => {
    const center = graphRef.current?.viewportCenter() ?? {
      x: layout.width / 2,
      y: layout.height / 2,
    };
    const occupied = layout.nodes.map((node) => ({
      ...(workspace?.nodes[node.id] ?? { x: node.x, y: node.y }),
      width: node.width,
      height: node.height,
    }));
    return findFreePosition(center, occupied);
  }, [layout, workspace]);

  const saveConfiguration = useCallback(
    async (nextConfig: ConfigData, nextWorkspace?: GraphWorkspace): Promise<boolean> => {
      setOperationError(null);
      try {
        await saveConfig(nextConfig);
        setConfig(nextConfig);
        if (nextWorkspace && !(await persistWorkspace(nextWorkspace))) return false;
        reloadGraph();
        return true;
      } catch (reason) {
        setOperationError((reason as Error).message);
        return false;
      }
    },
    [persistWorkspace, reloadGraph]
  );

  const createAgent = useCallback(
    (name: string, entry: AgentEntry) => {
      if (!config || !workspace) return;
      if (!name || !entry.command) {
        setOperationError("Agent name and command are required.");
        return;
      }
      if (config.agents[name]) {
        setOperationError(`Agent '${name}' already exists.`);
        return;
      }
      const nextConfig = { ...config, agents: { ...config.agents, [name]: entry } };
      const nextWorkspace = {
        ...workspace,
        nodes: { ...workspace.nodes, [`agent:${name}`]: freePosition() },
      };
      void saveConfiguration(nextConfig, nextWorkspace).then((saved) => {
        if (!saved) return;
        setAddOpen(false);
        setSelectedNodeId(`agent:${name}`);
        setPositionRevision((value) => value + 1);
      });
    },
    [config, freePosition, saveConfiguration, workspace]
  );

  const createRole = useCallback(
    (role: string, agent: string) => {
      if (!config || !workspace || !role || !agent) {
        setOperationError("Choose a supported role and configured agent.");
        return;
      }
      const nextConfig = {
        ...config,
        roles: { ...config.roles, [role]: { agent } },
      };
      const nextWorkspace = {
        ...workspace,
        nodes: { ...workspace.nodes, [`role:${role}`]: freePosition() },
      };
      void saveConfiguration(nextConfig, nextWorkspace).then((saved) => {
        if (!saved) return;
        setAddOpen(false);
        setSelectedNodeId(`role:${role}`);
        setPositionRevision((value) => value + 1);
      });
    },
    [config, freePosition, saveConfiguration, workspace]
  );

  const createVisual = useCallback(
    (kind: "group" | "note", label: string, text: string) => {
      if (!workspace || !label) {
        setOperationError("A label is required.");
        return;
      }
      const id = nextWorkspaceId(kind, label);
      const next = {
        ...workspace,
        customNodes: [...workspace.customNodes, { id, kind, label, ...(text ? { text } : {}) }],
        nodes: { ...workspace.nodes, [id]: freePosition() },
      };
      void persistWorkspace(next).then((saved) => {
        if (!saved) return;
        setAddOpen(false);
        setSelectedNodeId(id);
        setPositionRevision((value) => value + 1);
      });
    },
    [freePosition, persistWorkspace, workspace]
  );

  const connect = useCallback(
    (connection: Connection) => {
      if (!merged || !workspace || !config || !connection.source || !connection.target) return;
      const invalid = validateConnection(connection, allNodesById, merged.edges);
      if (invalid) {
        setOperationError(invalid);
        return;
      }
      const kind = connectionKind(
        allNodesById.get(connection.source),
        allNodesById.get(connection.target)
      );
      if (kind === "assignment") {
        const role = connection.source.slice("role:".length);
        const agent = connection.target.slice("agent:".length);
        void saveConfiguration({
          ...config,
          roles: { ...config.roles, [role]: { agent } },
        });
        return;
      }
      if (!kind) return;
      const id = `edge:${kind}:${Date.now().toString(36)}`;
      void persistWorkspace({
        ...workspace,
        edges: [
          ...workspace.edges,
          { id, source: connection.source, target: connection.target, kind },
        ],
      });
    },
    [allNodesById, config, merged, persistWorkspace, saveConfiguration, workspace]
  );

  const deleteSelection = useCallback(() => {
    if (!workspace || !config || !merged) return;
    if (selectedEdge) {
      if (!selectedEdge.editable) {
        setOperationError("This Factory-generated connection is read-only.");
        return;
      }
      if (selectedEdge.kind === "binds") {
        const role = selectedEdge.source.slice("role:".length);
        if (!window.confirm(`Remove the ${role} role assignment from Factory configuration?`))
          return;
        const roles = { ...config.roles };
        delete roles[role];
        const nodes = { ...workspace.nodes };
        delete nodes[`role:${role}`];
        const nextWorkspace = {
          ...workspace,
          nodes,
          edges: workspace.edges.filter(
            (edge) => edge.source !== `role:${role}` && edge.target !== `role:${role}`
          ),
        };
        void saveConfiguration({ ...config, roles }, nextWorkspace);
      } else {
        void persistWorkspace({
          ...workspace,
          edges: workspace.edges.filter((edge) => edge.id !== selectedEdge.id),
        });
      }
      setSelectedEdgeId(null);
      return;
    }
    if (!selectedNode) return;
    if (selectedNode.kind === "group" || selectedNode.kind === "note") {
      const nodes = { ...workspace.nodes };
      delete nodes[selectedNode.id];
      void persistWorkspace({
        ...workspace,
        nodes,
        customNodes: workspace.customNodes.filter((node) => node.id !== selectedNode.id),
        edges: workspace.edges.filter(
          (edge) => edge.source !== selectedNode.id && edge.target !== selectedNode.id
        ),
      });
      setSelectedNodeId(null);
      return;
    }
    if (selectedNode.kind === "role") {
      const role = selectedNode.id.slice("role:".length);
      if (!window.confirm(`Unassign the ${role} role from Factory configuration?`)) return;
      const roles = { ...config.roles };
      delete roles[role];
      const nodes = { ...workspace.nodes };
      delete nodes[selectedNode.id];
      const nextWorkspace = {
        ...workspace,
        nodes,
        edges: workspace.edges.filter(
          (edge) => edge.source !== selectedNode.id && edge.target !== selectedNode.id
        ),
      };
      void saveConfiguration({ ...config, roles }, nextWorkspace);
      setSelectedNodeId(null);
      return;
    }
    if (selectedNode.kind === "agent") {
      const name = selectedNode.id.slice("agent:".length);
      const affected = Object.entries(config.roles)
        .filter(([, entry]) => entry.agent === name)
        .map(([role]) => role);
      const detail = affected.length
        ? ` Assigned roles will also be removed: ${affected.join(", ")}.`
        : "";
      if (!window.confirm(`Remove agent '${name}' from Factory configuration?${detail}`)) return;
      const agents = { ...config.agents };
      delete agents[name];
      const roles = Object.fromEntries(
        Object.entries(config.roles).filter(([, entry]) => entry.agent !== name)
      );
      const nodes = { ...workspace.nodes };
      delete nodes[selectedNode.id];
      const nextWorkspace = {
        ...workspace,
        nodes,
        edges: workspace.edges.filter(
          (edge) => edge.source !== selectedNode.id && edge.target !== selectedNode.id
        ),
      };
      void saveConfiguration({ agents, roles }, nextWorkspace);
      setSelectedNodeId(null);
      return;
    }
    setOperationError("Run and task nodes are read-only Factory state.");
  }, [config, merged, persistWorkspace, saveConfiguration, selectedEdge, selectedNode, workspace]);

  const resetLayout = useCallback(() => {
    if (!workspace) return;
    if (
      !window.confirm("Discard all manually saved graph positions and recompute the neural layout?")
    )
      return;
    void persistWorkspace({ ...workspace, nodes: {} }).then((saved) => {
      if (!saved) return;
      setPositionRevision((value) => value + 1);
      window.setTimeout(() => graphRef.current?.fit(), 20);
    });
  }, [persistWorkspace, workspace]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (
        target?.matches("input, textarea, select") ||
        target?.closest(".agent-console") ||
        target?.isContentEditable
      ) {
        return;
      }
      if (event.key === "Escape") {
        setAddOpen(false);
        setSelectedNodeId(null);
        setSelectedEdgeId(null);
      } else if (event.key === "Delete" || event.key === "Backspace") {
        deleteSelection();
      } else if (event.key.toLowerCase() === "f" || event.key === "0") {
        event.preventDefault();
        graphRef.current?.fit();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [deleteSelection]);

  if (error) return <LoadError message={error} onRetry={reload} />;
  if (!data || !workspace || !config || !merged) return <Loading />;

  const empty = data.metadata.agents === 0;
  const visibleError = operationError ?? pollError;

  return (
    <div className="network-view">
      <GraphToolbar
        runOptions={runOptions}
        runFilter={runFilter}
        onRunFilter={setRunFilter}
        showTasks={effectiveShowTasks}
        onShowTasks={setShowTasks}
        showDependencies={showDependencies}
        onShowDependencies={setShowDependencies}
        live={live}
        onLive={(value) => {
          setLive(value);
          if (!value) setPollError(null);
        }}
        onAdd={() => setAddOpen((value) => !value)}
        onFit={() => graphRef.current?.fit()}
        onCenter={() => graphRef.current?.center()}
        onZoomOut={() => graphRef.current?.zoomOut()}
        onZoomIn={() => graphRef.current?.zoomIn()}
        onResetLayout={resetLayout}
        zoom={zoom}
      />
      <div className="network-stage">
        <AddNodeMenu
          open={addOpen}
          config={config}
          error={operationError}
          onClose={() => setAddOpen(false)}
          onCreateAgent={createAgent}
          onCreateRole={createRole}
          onCreateVisual={createVisual}
        />
        {visibleError && !addOpen && (
          <p className="net-inline-error" role="alert">
            {visibleError}
            <button
              onClick={() => {
                setOperationError(null);
                setPollError(null);
              }}
              aria-label="Dismiss graph error"
            >
              x
            </button>
          </p>
        )}

        {empty ? (
          <div className="empty net-empty">
            <p className="empty-title">No agents configured.</p>
            <p className="empty-body">Add your first coding agent to build the Factory graph.</p>
            <button className="button" onClick={() => setAddOpen(true)}>
              + Add agent
            </button>
          </div>
        ) : (
          <div className="net-layout">
            <div className="net-canvas">
              <AgentGraph
                ref={graphRef}
                layout={layout}
                nodesById={nodesById}
                edges={filteredEdges}
                positions={workspace.nodes}
                positionRevision={positionRevision}
                selectedNodeId={selectedNodeId}
                selectedEdgeId={selectedEdgeId}
                neighborIds={neighbors}
                activities={activities}
                onNodeSelect={(id) => {
                  setSelectedNodeId(id);
                  setSelectedEdgeId(null);
                }}
                onEdgeSelect={(id) => {
                  setSelectedEdgeId(id);
                  setSelectedNodeId(null);
                }}
                onBackgroundClick={() => {
                  setSelectedNodeId(null);
                  setSelectedEdgeId(null);
                }}
                onNodeDragStop={(id, position) => {
                  const next = {
                    ...workspace,
                    nodes: {
                      ...workspace.nodes,
                      [id]: { x: Math.round(position.x), y: Math.round(position.y) },
                    },
                  };
                  void persistWorkspace(next);
                }}
                onConnect={connect}
                isValidConnection={(connection) =>
                  "source" in connection &&
                  validateConnection(connection as Connection, allNodesById, merged.edges) === null
                }
                onZoomChange={setZoom}
              />
            </div>
            {selectedNode?.kind === "agent" ? (
              <AgentConsole
                agentName={selectedNode.id.slice("agent:".length)}
                meta={agentMeta(selectedNode)}
                activity={activities.get(selectedNode.id) ?? null}
                nodesById={allNodesById}
                onClose={() => setSelectedNodeId(null)}
                onDelete={deleteSelection}
                onConnect={(targetId) =>
                  connect({
                    source: selectedNode.id,
                    target: targetId,
                    sourceHandle: null,
                    targetHandle: null,
                  })
                }
              />
            ) : (
              <NodeInspector
                node={selectedNode}
                edge={selectedEdge}
                nodesById={allNodesById}
                onClose={() => {
                  setSelectedNodeId(null);
                  setSelectedEdgeId(null);
                }}
                onDelete={deleteSelection}
                onConnect={(targetId) => {
                  if (!selectedNode) return;
                  connect({
                    source: selectedNode.id,
                    target: targetId,
                    sourceHandle: null,
                    targetHandle: null,
                  });
                }}
              />
            )}
          </div>
        )}

        <div className="net-legend" aria-label="Graph legend">
          <span>Agents {data.metadata.agents}</span>
          <span>Roles {data.metadata.roles}</span>
          <span>Runs {data.metadata.runs}</span>
          <span>Tasks {data.metadata.tasks}</span>
          {STATE_ORDER.filter((state) => (data.metadata.tasks ? true : state === "running")).map(
            (state) => (
              <span key={state} className="net-legend-item">
                <span
                  className="net-status-dot"
                  style={{ backgroundColor: STATE_META[state].color }}
                />
                {state}
              </span>
            )
          )}
        </div>
      </div>
    </div>
  );
}
