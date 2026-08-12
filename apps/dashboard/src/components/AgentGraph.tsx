import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import type { GraphNode } from "../types";
import { taskMeta } from "../types";
import type { AgentActivity } from "../types";
import type { NetworkLayout } from "../networkLayout";
import { GraphEdge } from "./GraphEdge";
import { GraphNode as GraphNodeView } from "./GraphNode";

export interface AgentGraphHandle {
  fit: () => void;
  centerOn: (id: string) => void;
}

const ZOOM_MIN = 0.12;
const ZOOM_MAX = 3.2;

const EASE = "transform 0.4s cubic-bezier(0.22, 1, 0.36, 1)";

export const AgentGraph = forwardRef<
  AgentGraphHandle,
  {
    layout: NetworkLayout;
    nodesById: Map<string, GraphNode>;
    activeId: string | null;
    selectedId: string | null;
    neighborIds: string[];
    activities: Map<string, AgentActivity | null>;
    onNodeEnter: (id: string) => void;
    onNodeLeave: () => void;
    onNodeClick: (id: string) => void;
    onBackgroundClick: () => void;
  }
>(function AgentGraph(
  {
    layout,
    nodesById,
    activeId,
    selectedId,
    neighborIds,
    activities,
    onNodeEnter,
    onNodeLeave,
    onNodeClick,
    onBackgroundClick,
  },
  ref
) {
  const svgRef = useRef<SVGSVGElement>(null);
  const layoutRef = useRef(layout);
  layoutRef.current = layout;
  const viewRef = useRef({ x: 0, y: 0, s: 1 });
  const [view, setView] = useState({ x: 0, y: 0, s: 1 });
  const [easing, setEasing] = useState(false);

  const settle = useCallback(() => window.setTimeout(() => setEasing(false), 450), []);

  const reduceMotion =
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  useEffect(() => {
    const el = svgRef.current;
    if (!el) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const rect = el.getBoundingClientRect();
      const px = event.clientX - rect.left;
      const py = event.clientY - rect.top;
      const current = viewRef.current;
      const nextScale = Math.min(
        ZOOM_MAX,
        Math.max(ZOOM_MIN, current.s * Math.exp(-event.deltaY * 0.0014))
      );
      const wx = (px - current.x) / current.s;
      const wy = (py - current.y) / current.s;
      setView({ x: px - wx * nextScale, y: py - wy * nextScale, s: nextScale });
      setEasing(false);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const drag = useRef<{ px: number; py: number; moved: boolean } | null>(null);

  const onPointerDown = (event: React.PointerEvent<SVGSVGElement>) => {
    if (event.button !== 0) return;
    drag.current = { px: event.clientX, py: event.clientY, moved: false };
    (event.target as Element).setPointerCapture?.(event.pointerId);
  };

  const onPointerMove = (event: React.PointerEvent<SVGSVGElement>) => {
    const state = drag.current;
    if (!state) return;
    const dx = event.clientX - state.px;
    const dy = event.clientY - state.py;
    if (Math.abs(dx) + Math.abs(dy) > 3) state.moved = true;
    if (state.moved) {
      setView((current) => ({ ...current, x: current.x + dx, y: current.y + dy }));
      setEasing(false);
    }
    state.px = event.clientX;
    state.py = event.clientY;
  };

  const onPointerUp = (event: React.PointerEvent<SVGSVGElement>) => {
    const state = drag.current;
    drag.current = null;
    if (!state || state.moved) return;
    const el = svgRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const px = event.clientX - rect.left;
    const py = event.clientY - rect.top;
    const current = viewRef.current;
    const wx = (px - current.x) / current.s;
    const wy = (py - current.y) / current.s;
    // Walk boxes in render order (selected drawn last on top) and take the
    // topmost hit.
    const candidates =
      selectedId === null
        ? layoutRef.current.nodes
        : [...layoutRef.current.nodes].sort(
            (a, b) => Number(a.id === selectedId) - Number(b.id === selectedId)
          );
    let hit: (typeof layoutRef.current.nodes)[number] | null = null;
    for (let i = candidates.length - 1; i >= 0; i -= 1) {
      const pos = candidates[i];
      if (wx >= pos.x && wx <= pos.x + pos.width && wy >= pos.y && wy <= pos.y + pos.height) {
        hit = pos;
        break;
      }
    }
    if (hit) onNodeClick(hit.id);
    else onBackgroundClick();
  };

  useImperativeHandle(
    ref,
    () => ({
      fit() {
        const el = svgRef.current;
        if (!el) return;
        const cw = el.clientWidth;
        const ch = el.clientHeight;
        const lay = layoutRef.current;
        if (!lay || lay.nodes.length === 0) return;
        const margin = 56;
        const scale = Math.min((cw - margin * 2) / lay.width, (ch - margin * 2) / lay.height, 1.15);
        const s = Math.max(ZOOM_MIN, scale);
        viewRef.current = { x: (cw - lay.width * s) / 2, y: (ch - lay.height * s) / 2, s };
        setView(viewRef.current);
        setEasing(true);
        settle();
      },
      centerOn(id: string) {
        const el = svgRef.current;
        if (!el) return;
        const cw = el.clientWidth;
        const ch = el.clientHeight;
        const pos = layoutRef.current.nodes.find((node) => node.id === id);
        if (!pos) return;
        const { s } = viewRef.current;
        viewRef.current = { x: cw / 2 - pos.cx * s, y: ch / 2 - pos.cy * s, s };
        setView(viewRef.current);
        setEasing(true);
        settle();
      },
    }),
    [settle]
  );

  const isConnected = useCallback(
    (id: string) => {
      if (selectedId === null) return activeId !== null && id === activeId;
      return id === selectedId || neighborIds.includes(id);
    },
    [selectedId, activeId, neighborIds]
  );

  const ordered = useCallback(() => {
    const list = [...layout.nodes];
    if (selectedId !== null)
      list.sort((a, b) => Number(a.id === selectedId) - Number(b.id === selectedId));
    return list;
  }, [layout.nodes, selectedId]);

  const edges = layout.edges.map((edgePos) => {
    const target = nodesById.get(edgePos.target);
    const targetTask = target?.kind === "task" ? taskMeta(target) : null;
    const tone =
      targetTask && targetTask.state === "failed"
        ? "failed"
        : targetTask && targetTask.state === "blocked"
          ? "blocked"
          : "ok";
    const flowing = targetTask ? targetTask.state === "running" : false;
    const emphasized = isConnected(edgePos.source) || isConnected(edgePos.target);
    const dimmed = selectedId !== null && !emphasized;
    return (
      <GraphEdge
        key={`${edgePos.source}->${edgePos.target}`}
        path={edgePos.path}
        kind={edgePos.kind}
        tone={tone}
        flowing={flowing}
        emphasized={emphasized}
        dimmed={dimmed}
      />
    );
  });

  const nodes = ordered().map((pos) => {
    const node = nodesById.get(pos.id);
    if (!node) return null;
    const selected = selectedId === pos.id;
    const dimmed = selectedId === null ? false : !isConnected(pos.id) && !selected;
    return (
      <GraphNodeView
        key={pos.id}
        pos={pos}
        node={node}
        selected={selected}
        dimmed={dimmed}
        activity={node.kind === "agent" ? (activities.get(pos.id) ?? null) : null}
        onEnter={onNodeEnter}
        onLeave={onNodeLeave}
      />
    );
  });

  const ease = reduceMotion ? "none" : easing ? EASE : "none";

  return (
    <svg
      ref={svgRef}
      className="agent-canvas"
      role="img"
      aria-label="factory agent and task network"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => (drag.current = null)}
    >
      <defs>
        <pattern id="net-dots" width="24" height="24" patternUnits="userSpaceOnUse">
          <circle cx="1.5" cy="1.5" r="1" fill="var(--dot, rgba(148,163,184,0.06))" />
        </pattern>
        <marker
          id="net-arrow"
          viewBox="0 0 10 10"
          refX="8"
          refY="5"
          markerWidth="5.5"
          markerHeight="5.5"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 10 5 L 0 10 z" fill="context-stroke" />
        </marker>
      </defs>

      <rect className="agent-canvas-bg" width="100%" height="100%" fill="url(#net-dots)" />
      <rect className="agent-canvas-shade" width="100%" height="100%" />

      <g
        style={{
          transform: `translate(${view.x}px, ${view.y}px) scale(${view.s})`,
          transformOrigin: "0 0",
          transition: ease,
        }}
      >
        {edges}
        {nodes}
      </g>
    </svg>
  );
});
