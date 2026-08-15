import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Connection, XYPosition } from "@xyflow/react";
import {
  addRoleAssignment,
  cancelWorkflow,
  createRole,
  createWorkflow,
  deleteRole,
  fetchConfig,
  fetchGraph,
  fetchGraphWorkspace,
  fetchRoles,
  removeRoleAssignment,
  retryTask,
  saveConfig,
  saveGraphWorkspace,
  startWorkflow,
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
  RoleInfo,
  TaskState,
  WorkflowTeam,
} from "../types";
import { agentActivity, agentMeta, taskMeta, type AgentActivity } from "../types";
import { computeNetworkLayout, neighborsOf, type NetworkLayout } from "../networkLayout";
import { STATE_META } from "../state";
import { AddNodeMenu } from "./AddNodeMenu";
import { AgentConsole } from "./AgentConsole";
import { AgentGraph, type AgentGraphHandle } from "./AgentGraph";
import { GraphToolbar, type RunOption } from "./GraphToolbar";
import { NodeInspector } from "./NodeInspector";
import { RoleInspector } from "./RoleInspector";
import { WorkflowInspector } from "./WorkflowInspector";
import type { RoleFormValue } from "./RoleForm";

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
  const [roles, setRoles] = useState<RoleInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const [live, setLive] = useState(true);
  const [runFilter, setRunFilter] = useState<number | null>(null);
  const [showTasks, setShowTasks] = useState<boolean | null>(null);
  const [showDependencies, setShowDependencies] = useState(true);
  const [showRoles, setShowRoles] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [initialAddKind, setInitialAddKind] = useState<"workflow" | null>(null);
  const [zoom, setZoom] = useState(1);
  const [positionRevision, setPositionRevision] = useState(0);
  const graphRef = useRef<AgentGraphHandle>(null);
  const layoutCache = useRef<{ key: string; layout: NetworkLayout } | null>(null);

  const reload = useCallback(() => {
    Promise.all([fetchGraph(), fetchGraphWorkspace(), fetchConfig(), fetchRoles()])
      .then(([nextData, nextWorkspace, nextConfig, nextRoles]) => {
        setData(nextData);
        setWorkspace(nextWorkspace);
        setConfig(nextConfig);
        setRoles(nextRoles);
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

  const reloadRoles = useCallback(() => {
    fetchRoles()
      .then((next) => setRoles(next))
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
        if (node.kind === "role") {
          return showRoles;
        }
        return true;
      }),
    [effectiveShowTasks, merged, runFilter, showRoles]
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

  const createWorkflowNode = useCallback(
    async (objective: string, team: WorkflowTeam) => {
      if (!workspace) return;
      setOperationError(null);
      try {
        const run = await createWorkflow(objective, team);
        const nodeId = `run:${run.id}`;
        const nextWorkspace = {
          ...workspace,
          nodes: { ...workspace.nodes, [nodeId]: freePosition() },
        };
        await persistWorkspace(nextWorkspace);
        setAddOpen(false);
        setInitialAddKind(null);
        setSelectedNodeId(nodeId);
        setPositionRevision((value) => value + 1);
        reloadGraph();
      } catch (reason) {
        setOperationError((reason as Error).message);
      }
    },
    [freePosition, persistWorkspace, reloadGraph, workspace]
  );

  const startRun = useCallback(
    async (runId: number) => {
      await startWorkflow(runId);
      reloadGraph();
    },
    [reloadGraph]
  );

  const cancelRun = useCallback(
    async (runId: number) => {
      await cancelWorkflow(runId);
      reloadGraph();
    },
    [reloadGraph]
  );

  const retryFailedTask = useCallback(
    async (taskId: number) => {
      await retryTask(taskId);
      reloadGraph();
    },
    [reloadGraph]
  );

  const createRoleNode = useCallback(
    async (value: RoleFormValue) => {
      if (!workspace) return;
      setOperationError(null);
      try {
        const role = await createRole({
          id: value.id,
          name: value.name,
          description: value.description,
          executionClass: value.executionClass,
          instructions: value.instructions,
          agents: value.agents,
          preferredAgent: value.preferredAgent ?? undefined,
        });
        const nodeId = `role:${role.id}`;
        const nextWorkspace = {
          ...workspace,
          nodes: { ...workspace.nodes, [nodeId]: freePosition() },
        };
        await persistWorkspace(nextWorkspace);
        setAddOpen(false);
        setInitialAddKind(null);
        setSelectedNodeId(nodeId);
        setPositionRevision((revision) => revision + 1);
        reloadGraph();
        reloadRoles();
      } catch (reason) {
        setOperationError((reason as Error).message);
      }
    },
    [freePosition, persistWorkspace, reloadGraph, reloadRoles, workspace]
  );

  const assignCoreRole = useCallback(
    async (roleId: string, agent: string) => {
      if (!roleId || !agent) {
        setOperationError("Choose a role and a configured agent.");
        return;
      }
      setOperationError(null);
      try {
        await addRoleAssignment(roleId, agent);
        setAddOpen(false);
        setInitialAddKind(null);
        reloadGraph();
        reloadRoles();
      } catch (reason) {
        setOperationError((reason as Error).message);
      }
    },
    [reloadGraph, reloadRoles]
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
      if (!merged || !workspace || !connection.source || !connection.target) return;
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
        setOperationError(null);
        void addRoleAssignment(role, agent)
          .then(() => {
            reloadGraph();
            reloadRoles();
          })
          .catch((reason: Error) => setOperationError(reason.message));
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
    [allNodesById, merged, persistWorkspace, reloadGraph, reloadRoles, workspace]
  );

  const deleteSelection = useCallback(() => {
    if (!workspace || !config || !merged || !roles) return;
    if (selectedEdge) {
      if (!selectedEdge.editable) {
        setOperationError("This Factory-generated connection is read-only.");
        return;
      }
      if (selectedEdge.kind === "binds") {
        const role = selectedEdge.source.slice("role:".length);
        const agent = selectedEdge.target.slice("agent:".length);
        if (!window.confirm(`Remove the ${agent} assignment from the ${role} role?`)) return;
        setOperationError(null);
        void removeRoleAssignment(role, agent)
          .then(() => {
            reloadGraph();
            reloadRoles();
          })
          .catch((reason: Error) => setOperationError(reason.message));
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
      const roleId = selectedNode.id.slice("role:".length);
      const role = roles.find((candidate) => candidate.id === roleId);
      if (!role) return;
      if (role.kind === "core") {
        setOperationError(
          "Built-in core roles cannot be deleted; remove their assignments instead."
        );
        return;
      }
      if (!window.confirm(`Delete the ${role.name} custom role and its assignments?`)) return;
      setOperationError(null);
      void deleteRole(role.id)
        .then(() => {
          setSelectedNodeId(null);
          reloadGraph();
          reloadRoles();
        })
        .catch((reason: Error) => setOperationError(reason.message));
      return;
    }
    if (selectedNode.kind === "agent") {
      const name = selectedNode.id.slice("agent:".length);
      const affected = config.role_assignments
        .filter((entry) => entry.agent === name)
        .map((entry) => entry.role);
      const detail = affected.length
        ? ` Role assignments will also be removed: ${affected.join(", ")}.`
        : "";
      if (!window.confirm(`Remove agent '${name}' from Factory configuration?${detail}`)) return;
      const agents = { ...config.agents };
      delete agents[name];
      const role_assignments = config.role_assignments.filter((entry) => entry.agent !== name);
      const nodes = { ...workspace.nodes };
      delete nodes[selectedNode.id];
      const nextWorkspace = {
        ...workspace,
        nodes,
        edges: workspace.edges.filter(
          (edge) => edge.source !== selectedNode.id && edge.target !== selectedNode.id
        ),
      };
      void saveConfiguration({ ...config, agents, role_assignments }, nextWorkspace);
      setSelectedNodeId(null);
      return;
    }
    setOperationError("Run and task nodes are read-only Factory state.");
  }, [
    config,
    merged,
    persistWorkspace,
    reloadGraph,
    reloadRoles,
    roles,
    saveConfiguration,
    selectedEdge,
    selectedNode,
    workspace,
  ]);

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
  if (!data || !workspace || !config || !roles || !merged) return <Loading />;

  const empty = data.metadata.agents === 0;
  const visibleError = operationError ?? pollError;
  const selectedRole =
    selectedNode?.kind === "role"
      ? (roles.find((role) => role.id === selectedNode.id.slice("role:".length)) ?? null)
      : null;

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
        showRoles={showRoles}
        onShowRoles={setShowRoles}
        live={live}
        onLive={(value) => {
          setLive(value);
          if (!value) setPollError(null);
        }}
        onAdd={() => {
          setInitialAddKind(null);
          setAddOpen((value) => !value);
        }}
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
          roles={roles}
          error={operationError}
          initialKind={initialAddKind}
          onClose={() => {
            setAddOpen(false);
            setInitialAddKind(null);
          }}
          onCreateWorkflow={(objective, team) => void createWorkflowNode(objective, team)}
          onCreateAgent={createAgent}
          onCreateRole={(value) => void createRoleNode(value)}
          onAssignCoreRole={(roleId, agent) => void assignCoreRole(roleId, agent)}
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
            <p className="empty-body">
              Add agents, assign Planner / Worker / Reviewer, then create a workflow.
            </p>
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
            ) : selectedNode?.kind === "run" ? (
              <WorkflowInspector
                node={selectedNode}
                onClose={() => setSelectedNodeId(null)}
                onStart={startRun}
                onCancel={cancelRun}
                onRetry={retryFailedTask}
                onTeamUpdated={reloadGraph}
              />
            ) : selectedRole ? (
              <RoleInspector
                role={selectedRole}
                agents={Object.keys(config.agents).sort()}
                onClose={() => setSelectedNodeId(null)}
                onChanged={() => {
                  reloadGraph();
                  reloadRoles();
                }}
                onDelete={deleteSelection}
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
                onRetry={(taskId) => void retryFailedTask(taskId)}
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

        {!empty && data.metadata.runs === 0 && !addOpen && (
          <div className="net-workflow-cta">
            <strong>Create your first workflow</strong>
            <span>Plan work with the configured Planner, then inspect the task graph.</span>
            <button
              className="button button-primary"
              onClick={() => {
                setInitialAddKind("workflow");
                setAddOpen(true);
              }}
            >
              + Workflow
            </button>
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
