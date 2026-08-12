import { useEffect, useMemo, useState } from "react";
import { fetchGraph } from "../api";
import type { GraphData, GraphNodeKind, TaskState } from "../types";
import { computeNetworkLayout, neighborsOf } from "../networkLayout";
import { STATE_META } from "../state";
import { NetworkGraph } from "./NetworkGraph";
import { NodeInfo } from "./NodeInfo";

const KIND_COLOR: Record<GraphNodeKind, string> = {
  agent: "#3d7dfd",
  role: "#a78bfa",
  run: "#38bdf8",
  task: "#9ca3af",
};

const STATE_ORDER: TaskState[] = ["pending", "ready", "running", "blocked", "failed", "completed"];

function Loading() {
  return <p className="empty-title">Loading…</p>;
}

export function NetworkView() {
  const [data, setData] = useState<GraphData | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    fetchGraph()
      .then(setData)
      .catch((err: Error) => setError(err.message));
  }, []);

  const nodesById = useMemo(() => new Map(data?.nodes.map((n) => [n.id, n]) ?? []), [data]);
  const layout = useMemo(() => {
    if (!data) return null;
    return computeNetworkLayout(data.nodes, data.edges);
  }, [data]);

  const activeId = selectedId ?? focusId;
  const neighbors = useMemo(
    () => (layout && activeId ? neighborsOf(layout.nodes, layout.edges, activeId) : []),
    [layout, activeId]
  );
  const activeNode = activeId ? (nodesById.get(activeId) ?? null) : null;

  const clearSelection = () => {
    setSelectedId(null);
    setFocusId(null);
  };

  return (
    <div className="network-view">
      {error && <p className="error">{error}</p>}
      {!error && data === null && <Loading />}

      {!error && data !== null && data.nodes.length === 0 && (
        <div className="empty">
          <p className="empty-title">No factory network to show</p>
          <p className="empty-body">
            Configure agents in <code>.factory/config.toml</code> and start a run with{" "}
            <code>factory run</code> to see the network here.
          </p>
        </div>
      )}

      {!error && data !== null && layout !== null && data.nodes.length > 0 && (
        <>
          <div className="net-summary">
            <span className="net-summary-item">
              <span className="net-summary-label">Agents</span>
              {data.metadata.agents}{" "}
              {data.metadata.missingAgents > 0 && (
                <span className="net-bad">({data.metadata.missingAgents} missing)</span>
              )}
            </span>
            <span className="net-summary-item">
              <span className="net-summary-label">Roles</span>
              {data.metadata.roles}
            </span>
            <span className="net-summary-item">
              <span className="net-summary-label">Runs</span>
              {data.metadata.runs}
            </span>
            <span className="net-summary-item">
              <span className="net-summary-label">Tasks</span>
              {data.metadata.tasks}
            </span>
          </div>

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

          <div className="net-layout">
            <div className="graph-wrap network-graph-wrap">
              <NetworkGraph
                layout={layout}
                nodesById={nodesById}
                activeId={activeId}
                neighborIds={neighbors}
                onNodeEnter={setFocusId}
                onNodeLeave={() => setFocusId(null)}
                onNodeClick={(id) => setSelectedId(selectedId === id ? null : id)}
                onBackgroundClick={() => setSelectedId(null)}
              />
            </div>
            <NodeInfo node={activeNode} nodesById={nodesById} onClose={clearSelection} />
          </div>
        </>
      )}
    </div>
  );
}
