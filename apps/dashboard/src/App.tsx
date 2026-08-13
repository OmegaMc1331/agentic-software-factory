import { useCallback, useEffect, useState } from "react";
import { fetchRun, fetchRuns } from "./api";
import { NetworkView } from "./components/NetworkView";
import { RunDetailView } from "./components/RunDetailView";
import { RunList } from "./components/RunList";
import { SettingsView } from "./components/Settings";
import type { RunDetail, RunSummary } from "./types";

type View =
  { name: "runs" } | { name: "network" } | { name: "settings" } | { name: "run"; id: number };
type LoadState = "loading" | "ready" | "error";

function viewFromHash(): View {
  const runMatch = window.location.hash.match(/^#\/runs\/(\d+)$/);
  if (runMatch) return { name: "run", id: Number(runMatch[1]) };
  if (window.location.hash.startsWith("#/network")) return { name: "network" };
  if (window.location.hash.startsWith("#/settings")) return { name: "settings" };
  return { name: "runs" };
}

function hashFor(view: View): string {
  if (view.name === "run") return `#/runs/${view.id}`;
  if (view.name === "network") return "#/network";
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
  { view: "settings", label: "Settings" },
];

export default function App() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [runsState, setRunsState] = useState<LoadState>("loading");
  const [runsError, setRunsError] = useState("");
  const [detail, setDetail] = useState<RunDetail | null>(null);
  const [view, setView] = useState<View>(viewFromHash);
  const [detailError, setDetailError] = useState<string | null>(null);

  const loadRun = useCallback((id: number) => {
    setDetail(null);
    setDetailError(null);
    fetchRun(id)
      .then(setDetail)
      .catch((err: Error) => setDetailError(err.message));
  }, []);

  const loadRuns = useCallback(() => {
    setRunsState("loading");
    setRunsError("");
    fetchRuns()
      .then((nextRuns) => {
        setRuns(nextRuns);
        setRunsState("ready");
      })
      .catch((err: Error) => {
        setRunsError(err.message);
        setRunsState("error");
      });
  }, []);

  useEffect(() => {
    loadRuns();
  }, [loadRuns]);

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
        ) : view.name === "settings" ? (
          <SettingsView />
        ) : (
          <>
            {view.name === "run" && detailError && <p className="error">{detailError}</p>}
            {view.name === "run" && !detailError && detail !== null && (
              <RunDetailView detail={detail} onBack={() => navigate({ name: "runs" })} />
            )}
            {view.name === "run" && !detailError && detail === null && <Loading />}
            {view.name === "runs" && runsState === "loading" && <Loading />}
            {view.name === "runs" && runsState === "error" && (
              <RunsError message={runsError} onRetry={loadRuns} />
            )}
            {view.name === "runs" && runsState === "ready" && runs !== null && (
              <RunList runs={runs} onSelect={(id) => navigate({ name: "run", id })} />
            )}
          </>
        )}
      </main>
    </div>
  );
}
