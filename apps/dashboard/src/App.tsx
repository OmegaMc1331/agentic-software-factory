import { useCallback, useEffect, useState } from "react";
import { fetchRun, fetchRuns } from "./api";
import { RunDetailView } from "./components/RunDetailView";
import { RunList } from "./components/RunList";
import type { RunDetail, RunSummary } from "./types";

function runIdFromHash(): number | null {
  const match = window.location.hash.match(/^#\/runs\/(\d+)$/);
  return match ? Number(match[1]) : null;
}

function Loading() {
  return <p className="empty-title">Loading…</p>;
}

export default function App() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadRun = useCallback((id: number) => {
    setDetail(null);
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
    const applyHash = () => {
      const id = runIdFromHash();
      if (id !== null) {
        loadRun(id);
      } else {
        setDetail(null);
      }
    };
    applyHash();
    window.addEventListener("hashchange", applyHash);
    return () => window.removeEventListener("hashchange", applyHash);
  }, [loadRun]);

  const openRun = (id: number) => {
    window.location.hash = `#/runs/${id}`;
  };

  const back = () => {
    window.location.hash = "#/";
  };

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <span className="brand-name">Agentic Software Factory</span>
        </div>
        <nav className="nav">
          <button className="nav-link nav-active" onClick={back}>
            Runs
          </button>
        </nav>
      </header>

      <main className="content">
        {error && <p className="error">{error}</p>}
        {!error && runs === null && <Loading />}
        {!error && runs !== null && detail === null && (
          <RunList runs={runs} onSelect={openRun} />
        )}
        {!error && detail !== null && (
          <RunDetailView detail={detail} onBack={back} />
        )}
      </main>
    </div>
  );
}