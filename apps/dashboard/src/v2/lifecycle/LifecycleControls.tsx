import { useState } from "react";
import { request } from "../../api";
import { Button } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import { projectPath } from "../projects/projectApi";
import { useData, refreshData } from "../workspace/production/data";
import { browserPresence } from "./presence";
import type { MissionLifecycle, SessionLifecycle } from "./types";

async function command(project: string, path: string, generation: number) {
  return request(projectPath(project, path), {
    method: "POST",
    headers: { "Content-Type": "application/json", ...browserPresence.headers() },
    body: JSON.stringify({ expectedGeneration: generation, idempotencyKey: crypto.randomUUID() }),
  });
}
export function WorkspaceLifecycleControls({
  projectId,
  sessionId,
}: {
  projectId: string;
  sessionId: string;
}) {
  if (!/^p_[0-9a-f]{32}$/.test(projectId) || !/^s_[0-9a-f]{32}$/.test(sessionId)) return null;
  return (
    <WorkspaceLifecycle
      key={`${projectId}:${sessionId}`}
      projectId={projectId}
      sessionId={sessionId}
    />
  );
}
function WorkspaceLifecycle({ projectId, sessionId }: { projectId: string; sessionId: string }) {
  const path = `workspace-sessions/${sessionId}/lifecycle`;
  const data = useData<SessionLifecycle>(projectId, path);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const change = async (close: boolean) => {
    if (!data.value) return;
    setBusy(true);
    setError("");
    try {
      await command(
        projectId,
        `workspace-sessions/${sessionId}/${close ? "close" : "reopen"}`,
        data.value.generation
      );
      setConfirm(false);
      await refreshData(projectId, path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button
        tone="quiet"
        disabled={busy || !data.value}
        onClick={() => (data.value?.closed ? void change(false) : setConfirm(true))}
      >
        {data.value?.closed ? "Reopen session" : "Close session"}
      </Button>
      {error && <span role="alert">{error}</span>}
      {data.value?.closed && (
        <span role="status">
          {data.value.missions.some((m) => m.state === "recovery_required")
            ? "Session close requires recovery"
            : data.value.missions.some((m) => ["suspend_requested", "quiescing"].includes(m.state))
              ? "Session close requested; suspension in progress"
              : "Session closed"}
        </span>
      )}
      <Dialog
        open={confirm}
        title="Close workspace session"
        onClose={() => setConfirm(false)}
        footer={
          <Button disabled={busy} onClick={() => void change(true)}>
            Close and suspend missions
          </Button>
        }
      >
        <p>
          New task scheduling will stop for this session. Active agents will be reconciled and their
          work retained. Conversations, files and layout stay available. Development terminals,
          shared language servers and debuggers keep their existing independent lifecycles.
        </p>
        <ul>
          {data.value?.missions
            .filter((m) => m.state !== "settled")
            .map((m) => (
              <li key={m.runId}>
                Mission #{m.runId}: {m.state.replaceAll("_", " ")}
              </li>
            ))}
        </ul>
        <p>Reopen this session and explicitly resume each eligible mission to continue.</p>
      </Dialog>
    </>
  );
}

export function MissionLifecycleControls({
  projectId,
  runId,
}: {
  projectId: string;
  runId: number;
}) {
  if (!/^p_[0-9a-f]{32}$/.test(projectId) || !Number.isSafeInteger(runId) || runId <= 0)
    return null;
  return <MissionLifecycle key={`${projectId}:${runId}`} projectId={projectId} runId={runId} />;
}
function MissionLifecycle({ projectId, runId }: { projectId: string; runId: number }) {
  const path = `runs/${runId}/lifecycle`;
  const data = useData<{
    lifecycle: MissionLifecycle | null;
    legacy: boolean;
    processes?: Array<{
      sessionId: number;
      status: string;
      metadata: { processId?: number; processTreeCleanupConfirmed: boolean } | null;
    }>;
  }>(projectId, path);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [checkpoint, setCheckpoint] = useState<unknown>();
  const life = data.value?.lifecycle;
  const act = async (action: "suspend" | "resume") => {
    if (!life) return;
    setBusy(true);
    setError("");
    try {
      await command(projectId, `runs/${runId}/${action}`, life.generation);
      await refreshData(projectId, path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  if (!life) return null;
  return (
    <section className="asf-mission-lifecycle" aria-label={`Mission ${runId} lifecycle`}>
      <p role="status">
        {data.error ? "Last confirmed execution" : "Execution"}: {life.state.replaceAll("_", " ")}
      </p>
      {life.reason && <p>Reason: {life.reason.replaceAll("_", " ")}</p>}
      {life.checkpointId && (
        <p>
          Checkpoint #{life.checkpointId} · generation {life.generation}
        </p>
      )}
      <p>
        {data.value?.processes?.filter((p) => p.status === "running" || p.status === "active")
          .length ?? 0}{" "}
        recorded active agent processes
      </p>
      {data.error && (
        <InlineAlert tone="warning" title="Lifecycle connection unavailable">
          {data.error}
        </InlineAlert>
      )}
      {life.problem && (
        <InlineAlert tone="warning" title="Recovery requires inspection">
          {life.problem}
        </InlineAlert>
      )}
      {error && (
        <InlineAlert tone="danger" title="Lifecycle request refused">
          {error}
        </InlineAlert>
      )}
      {life.state === "active" && (
        <Button disabled={busy} onClick={() => void act("suspend")}>
          Suspend mission
        </Button>
      )}
      {["suspended", "recovery_required"].includes(life.state) && (
        <Button disabled={busy} onClick={() => void act("resume")}>
          Validate and resume
        </Button>
      )}
      {life.checkpointId && (
        <Button
          tone="quiet"
          onClick={() =>
            void request(projectPath(projectId, `runs/${runId}/checkpoints`))
              .then(setCheckpoint)
              .catch((e) => setError(String(e)))
          }
        >
          Inspect checkpoints
        </Button>
      )}
      {checkpoint !== undefined && (
        <details open>
          <summary>Checkpoint metadata</summary>
          <pre>{JSON.stringify(checkpoint, null, 2)}</pre>
        </details>
      )}
    </section>
  );
}
