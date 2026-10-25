import { useCallback, useEffect, useRef, useState } from "react";
import { fetchRun, fetchRunsPage, RUN_PAGE_SIZE } from "./api";
import { NetworkView } from "./components/NetworkView";
import { PerformanceView } from "./components/PerformanceView";
import { RunDetailView } from "./components/RunDetailView";
import { RunList } from "./components/RunList";
import { SettingsView } from "./components/Settings";
import type { RunDetail, RunPage } from "./types";

type View =
  | { name: "runs" }
  | { name: "network" }
  | { name: "performance"; agent: string | null }
  | { name: "settings" }
  | { name: "run"; id: number };
type LoadState = "loading" | "ready" | "error";

function viewFromHash(): View {
  const runMatch = window.location.hash.match(/^#\/runs\/(\d+)$/);
  if (runMatch) return { name: "run", id: Number(runMatch[1]) };
  const performanceMatch = window.location.hash.match(/^#\/performance(?:\/([^/]+))?$/);
  if (performanceMatch) {
    return {
      name: "performance",
      agent: performanceMatch[1] ? decodeURIComponent(performanceMatch[1]) : null,
    };
  }
  if (window.location.hash.startsWith("#/network")) return { name: "network" };
  if (window.location.hash.startsWith("#/settings")) return { name: "settings" };
  return { name: "runs" };
}

function hashFor(view: View): string {
  if (view.name === "run") return `#/runs/${view.id}`;
  if (view.name === "network") return "#/network";
  if (view.name === "performance") {
    return view.agent ? `#/performance/${encodeURIComponent(view.agent)}` : "#/performance";
  }
  if (view.name === "settings") return "#/settings";
  return "#/";
}

function Loading() {
  return <p className="empty-title">Loading…</p>;
}

function RunsError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="empty" role="alert">
      <p className="empty-title">Could not connect to the Factory API.</p>
      <p className="empty-body">
        Check that <code>factory start</code> is running.
      </p>
      <p className="error">{message}</p>
      <div className="empty-actions">
        <button className="button" onClick={onRetry}>
          Retry
        </button>
      </div>
    </div>
  );
}

const NAV: { view: View["name"]; label: string }[] = [
  { view: "runs", label: "Runs" },
  { view: "network", label: "Agent Graph" },
  { view: "performance", label: "Performance" },
  { view: "settings", label: "Settings" },
];

export default function App() {
  const [runsPage, setRunsPage] = useState<RunPage | null>(null);
  const [runsState, setRunsState] = useState<LoadState>("loading");
  const [runsError, setRunsError] = useState("");
  const [pageCursors, setPageCursors] = useState<(number | null)[]>([null]);
  const [pageIndex, setPageIndex] = useState(0);
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [view, setView] = useState<View>(viewFromHash);
  const [detailError, setDetailError] = useState<string | null>(null);
  const runsRequestRef = useRef(0);
  const runsAbortRef = useRef<AbortController | null>(null);

  const loadRun = useCallback((id: number) => {
    setDetail(null);
    setDetailError(null);
    fetchRun(id)
      .then(setDetail)
      .catch((err: Error) => setDetailError(err.message));
  }, []);

  const loadRuns = useCallback((cursor: number | null) => {
    runsAbortRef.current?.abort();
    const controller = new AbortController();
    runsAbortRef.current = controller;
    const requestId = ++runsRequestRef.current;
    setRunsState("loading");
    setRunsError("");
    fetchRunsPage({ limit: RUN_PAGE_SIZE, before: cursor }, { signal: controller.signal })
      .then((page) => {
        if (requestId !== runsRequestRef.current) return;
        setRunsPage(page);
        setRunsState("ready");
      })
      .catch((err: Error) => {
        if (requestId !== runsRequestRef.current) return;
        setRunsError(err.message);
        setRunsState("error");
      });
  }, []);

  useEffect(() => {
    loadRuns(null);
    return () => runsAbortRef.current?.abort();
  }, [loadRuns]);

  const currentCursor = pageCursors[pageIndex] ?? null;

  const goNext = () => {
    const nextCursor = runsPage?.nextCursor;
    if (!runsPage?.hasMore || nextCursor == null) return;
    const nextCursors = pageCursors.slice(0, pageIndex + 1);
    nextCursors.push(nextCursor);
    setPageCursors(nextCursors);
    setPageIndex(nextCursors.length - 1);
    loadRuns(nextCursor);
  };

  const goPrevious = () => {
    if (pageIndex === 0) return;
    const previousIndex = pageIndex - 1;
    setPageIndex(previousIndex);
    loadRuns(pageCursors[previousIndex] ?? null);
  };

  useEffect(() => {
    const applyHash = () => setView(viewFromHash());
    applyHash();
    window.addEventListener("hashchange", applyHash);
    return () => window.removeEventListener("hashchange", applyHash);
  }, []);

  useEffect(() => {
    if (view.name === "run") {
      loadRun(view.id);
    } else {
      setDetail(null);
    }
  }, [view, loadRun]);

  const navigate = (next: View) => {
    setDetailError(null);
    window.location.hash = hashFor(next);
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <span className="brand-name">Agentic Software Factory</span>
        </div>
        <nav className="nav">
          {NAV.map((item) => (
            <button
              key={item.view}
              className={view.name === item.view ? "nav-link nav-active" : "nav-link"}
              onClick={() =>
                navigate(
                  item.view === "runs"
                    ? { name: "runs" }
                    : item.view === "network"
                      ? { name: "network" }
                      : item.view === "performance"
                        ? { name: "performance", agent: null }
                        : { name: "settings" }
                )
              }
            >
              {item.label}
            </button>
          ))}
        </nav>
      </header>

      <main className={view.name === "network" ? "content content--wide" : "content"}>
        {view.name === "network" ? (
          <NetworkView />
        ) : view.name === "performance" ? (
          <PerformanceView key={view.agent ?? ""} initialAgent={view.agent} />
        ) : view.name === "settings" ? (
          <SettingsView />
        ) : (
          <>
            {view.name === "run" && detailError && <p className="error">{detailError}</p>}
            {view.name === "run" && !detailError && detail !== null && (
              <RunDetailView detail={detail} onBack={() => navigate({ name: "runs" })} />
            )}
            {view.name === "run" && !detailError && detail === null && <Loading />}
            {view.name === "runs" && runsState === "loading" && runsPage === null && <Loading />}
            {view.name === "runs" && runsState === "error" && (
              <RunsError message={runsError} onRetry={() => loadRuns(currentCursor)} />
            )}
            {view.name === "runs" && runsState !== "error" && runsPage !== null && (
              <RunList
                runs={runsPage.items}
                onSelect={(id) => navigate({ name: "run", id })}
                pageIndex={pageIndex}
                hasMore={runsPage.hasMore}
                loading={runsState === "loading"}
                onNext={goNext}
                onPrevious={goPrevious}
              />
            )}
          </>
        )}
      </main>
    </div>
  );
}
