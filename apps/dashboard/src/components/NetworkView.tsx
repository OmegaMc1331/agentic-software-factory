import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchGraph } from "../api";
import type { GraphData, GraphNode, GraphNodeKind, TaskState } from "../types";
import { agentActivity, taskMeta, type AgentActivity } from "../types";
import { computeNetworkLayout, neighborsOf } from "../networkLayout";
import { STATE_META } from "../state";
import { AgentGraph, type AgentGraphHandle } from "./AgentGraph";
import { GraphToolbar, type RunOption } from "./GraphToolbar";
import { NodeInspector } from "./NodeInspector";

const KIND_COLOR: Record<GraphNodeKind, string> = {
  agent: "#3d7dfd",
  role: "#a78bfa",
  run: "#38bdf8",
  task: "#9ca3af",
};

const STATE_ORDER: TaskState[] = ["pending", "ready", "running", "blocked", "failed", "completed"];

const POLL_MS = 3000;

function Loading() {
  return <p className="empty-title">Loading…</p>;
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

export function NetworkView() {
  const [data, setData] = useState<GraphData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [live, setLive] = useState(true);
  const [runFilter, setRunFilter] = useState<number | null>(null);
  const [showTasks, setShowTasks] = useState<boolean | null>(null);
  const [showDependencies, setShowDependencies] = useState(true);

  const graphRef = useRef<AgentGraphHandle>(null);

  const reload = useCallback(() => {
    fetchGraph()
      .then((nextData) => {
        setData(nextData);
        setError(null);
      })
      .catch((err: Error) => setError(err.message));
  }, []);

  useEffect(() => {
    reload();
  }, [reload]);

  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(reload, POLL_MS);
    return () => window.clearInterval(timer);
  }, [live, reload]);

  const allNodesById = useMemo(() => new Map((data?.nodes ?? []).map((n) => [n.id, n])), [data]);

  const activities = useMemo(() => {
    const map = new Map<string, AgentActivity | null>();
    if (!data) return map;
    for (const node of data.nodes) {
      if (node.kind === "agent") map.set(node.id, agentActivity(node.id, allNodesById, data.edges));
    }
    return map;
  }, [data, allNodesById]);

  const runOptions: RunOption[] = useMemo(
    () =>
      (data?.nodes ?? [])
        .filter((n): n is GraphNode & { kind: "run" } => n.kind === "run")
        .map((run) => ({
          id: Number(run.id.slice(4)),
          label: run.label,
        })),
    [data]
  );

  const effectiveShowTasks =
    runFilter !== null ? true : (showTasks ?? (data?.metadata.tasks ?? 0) > 0);

  const filteredNodes = useMemo(() => {
    if (!data) return [];
    return data.nodes.filter((node) => {
      if (node.kind === "task") {
        if (!effectiveShowTasks) return false;
        if (runFilter !== null && taskMeta(node).runId !== runFilter) return false;
        return true;
      }
      if (node.kind === "run") {
        return runFilter === null || Number(node.id.slice(4)) === runFilter;
      }
      return true;
    });
  }, [data, effectiveShowTasks, runFilter]);

  const visibleIds = useMemo(() => new Set(filteredNodes.map((n) => n.id)), [filteredNodes]);

  const filteredEdges = useMemo(() => {
    if (!data) return [];
    return data.edges.filter(
      (edge) =>
        (edge.kind !== "depends" || showDependencies) &&
        visibleIds.has(edge.source) &&
        visibleIds.has(edge.target)
    );
  }, [data, visibleIds, showDependencies]);

  const nodesById = useMemo(() => new Map(filteredNodes.map((n) => [n.id, n])), [filteredNodes]);

  const layout = useMemo(
    () => computeNetworkLayout(filteredNodes, filteredEdges),
    [filteredNodes, filteredEdges]
  );

  const signature = useMemo(
    () =>
      filteredNodes.length === 0
        ? ""
        : filteredNodes
            .map((n) => n.id)
            .sort()
            .join("|"),
    [filteredNodes]
  );

  const lastSignature = useRef<string | null>(null);
  useEffect(() => {
    if (signature !== lastSignature.current && filteredNodes.length > 0) {
      lastSignature.current = signature;
      graphRef.current?.fit();
    }
  }, [signature, filteredNodes]);

  const activeId = selectedId ?? focusId;
  const neighbors = useMemo(() => {
    if (!activeId) return [];
    return neighborsOf(layout.nodes, layout.edges, activeId);
  }, [layout, activeId]);
  const activeNode = activeId ? (nodesById.get(activeId) ?? null) : null;

  useEffect(() => {
    if (selectedId !== null) graphRef.current?.centerOn(selectedId);
  }, [selectedId]);

  const clearSelection = useCallback(() => {
    setSelectedId(null);
    setFocusId(null);
  }, []);

  const counts = useMemo(
    () => ({
      runs: runOptions.length,
      agents: data?.metadata.agents ?? 0,
      tasks: data?.metadata.tasks ?? 0,
    }),
    [runOptions, data]
  );

  if (error) {
    return <LoadError message={error} onRetry={reload} />;
  }

  if (data === null) {
    return <Loading />;
  }

  if (data.metadata.agents === 0) {
    return (
      <div className="empty net-empty">
        <p className="empty-title">No agents configured.</p>
        <p className="empty-body">
          Configure agents in <code>.factory/config.toml</code> — the network has nothing to
          visualize yet.
        </p>
      </div>
    );
  }

  const metadata = data.metadata;

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
        onLive={setLive}
        onFit={() => graphRef.current?.fit()}
        onCenter={() => {
          if (selectedId) graphRef.current?.centerOn(selectedId);
          else graphRef.current?.fit();
        }}
        counts={counts}
      />

      <div className="net-legend">
        <span className="net-legend-group">
          {(["agent", "role", "run", "task"] as GraphNodeKind[]).map((kind) => (
            <span key={kind} className="net-legend-item">
              <span className="net-status-dot" style={{ backgroundColor: KIND_COLOR[kind] }} />
              {kind}
            </span>
          ))}
        </span>
        <span className="net-legend-divider" />
        <span className="net-legend-group">
          {STATE_ORDER.map((state) => (
            <span key={state} className="net-legend-item">
              <span
                className="net-status-dot"
                style={{ backgroundColor: STATE_META[state].color }}
              />
              {state}
            </span>
          ))}
          <span className="net-legend-item">
            <span className="net-status-dot net-status-dot--hollow" />
            missing agent
          </span>
        </span>
      </div>

      {metadata.runs === 0 && (
        <div className="net-hint">
          No run yet — start one with <code>factory run "objective"</code> to see tasks here.
        </div>
      )}

      <div className="net-layout">
        <div className="net-canvas">
          <AgentGraph
            ref={graphRef}
            layout={layout}
            nodesById={nodesById}
            activeId={activeId}
            selectedId={selectedId}
            neighborIds={neighbors}
            activities={activities}
            onNodeEnter={setFocusId}
            onNodeLeave={() => setFocusId(null)}
            onNodeClick={(id) => setSelectedId((current) => (current === id ? null : id))}
            onBackgroundClick={() => setSelectedId(null)}
          />
        </div>
        <NodeInspector
          node={activeNode}
          nodesById={allNodesById}
          activity={activeNode?.kind === "agent" ? (activities.get(activeNode.id) ?? null) : null}
          onClose={clearSelection}
        />
      </div>
    </div>
  );
}
