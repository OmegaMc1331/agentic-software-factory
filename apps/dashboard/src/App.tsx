import { useCallback, useEffect, useState } from "react";
import { fetchRun, fetchRuns } from "./api";
import { NetworkView } from "./components/NetworkView";
import { RunDetailView } from "./components/RunDetailView";
import { RunList } from "./components/RunList";
import type { RunDetail, RunSummary } from "./types";

type View = { name: "runs" } | { name: "network" } | { name: "run"; id: number };

function viewFromHash(): View {
  const runMatch = window.location.hash.match(/^#\/runs\/(\d+)$/);
  if (runMatch) return { name: "run", id: Number(runMatch[1]) };
  if (window.location.hash.startsWith("#/network")) return { name: "network" };
  return { name: "runs" };
}

function hashFor(view: View): string {
  if (view.name === "run") return `#/runs/${view.id}`;
  if (view.name === "network") return "#/network";
  return "#/";
}

function Loading() {
  return <p className="empty-title">Loading…</p>;
}

export default function App() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [view, setView] = useState<View>(viewFromHash);
  const [error, setError] = useState<string | null>(null);

  const loadRun = useCallback((id: number) => {
    setDetail(null);
    setError(null);
    fetchRun(id)
      .then(setDetail)
      .catch((err: Error) => setError(err.message));
  }, []);

  useEffect(() => {
    fetchRuns()
      .then(setRuns)
      .catch((err: Error) => setError(err.message));
  }, []);

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
    setError(null);
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
          <button
            className={view.name === "network" ? "nav-link" : "nav-link nav-active"}
            onClick={() => navigate({ name: "runs" })}
          >
            Runs
          </button>
          <button
            className={view.name === "network" ? "nav-link nav-active" : "nav-link"}
            onClick={() => navigate({ name: "network" })}
          >
            Agent Graph
          </button>
        </nav>
      </header>

      <main className="content">
        {view.name === "network" ? (
          <NetworkView />
        ) : (
          <>
            {error && <p className="error">{error}</p>}
            {view.name === "run" && !error && detail !== null && (
              <RunDetailView detail={detail} onBack={() => navigate({ name: "runs" })} />
            )}
            {view.name === "run" && !error && detail === null && <Loading />}
            {view.name === "runs" && !error && runs === null && <Loading />}
            {view.name === "runs" && !error && runs !== null && (
              <RunList runs={runs} onSelect={(id) => navigate({ name: "run", id })} />
            )}
          </>
        )}
      </main>
    </div>
  );
}
