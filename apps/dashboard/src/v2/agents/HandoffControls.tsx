import { useState } from "react";
import type { RunDetail } from "../../types";
import { Button, Select, Textarea } from "../design-system/primitives/Controls";
import { refreshData, useData } from "../workspace/production/data";
import type { DockSession } from "../workspace/production/controller";
import { requestHandoff } from "./client";
import type { ExecutionSnapshot, Handoff, Profile } from "./types";
import "./agents.css";

export function SessionConfiguration({
  projectId,
  sessionId,
}: {
  projectId: string;
  sessionId: number;
}) {
  const snapshot = useData<ExecutionSnapshot | null>(
    projectId,
    `agent-sessions/${sessionId}/execution-snapshot`
  );
  const observation = useData<{
    model: string | null;
    source: string;
    successfulExecution: boolean;
  } | null>(projectId, `agent-sessions/${sessionId}/model-observation`);
  if (!snapshot.value?.profile) return snapshot.error ? <p role="alert">{snapshot.error}</p> : null;
  const value = snapshot.value;
  return (
    <details className="asf-execution-snapshot">
      <summary>
        {value.profile.name} · revision {value.profile.revision} · {value.providerKind} · requested{" "}
        {value.requestedModel ?? "provider default"}
      </summary>
      <p>
        Immutable launch configuration. A requested model is not proof of provider availability.
      </p>
      <p>
        Observed model: {observation.value?.model ?? "Not reported"} ·{" "}
        {observation.value?.source ?? "Awaiting execution"}
      </p>
      <pre tabIndex={0}>{JSON.stringify(value, null, 2)}</pre>
    </details>
  );
}

function HandoffForm({
  projectId,
  runId,
  dock,
}: {
  projectId: string;
  runId: number;
  dock: DockSession;
}) {
  const profiles = useData<Profile[]>(projectId, "agent-profiles");
  const records = useData<Handoff[]>(projectId, `runs/${runId}/agent-handoffs`);
  const detail = useData<RunDetail>(projectId, `runs/${runId}`);
  const [taskId, setTaskId] = useState("");
  const [target, setTarget] = useState("");
  const [reason, setReason] = useState("");
  const [key, setKey] = useState(() => crypto.randomUUID());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const tasks = detail.value?.tasks ?? [];
  const task = tasks.find((t) => t.id === Number(taskId));
  const role = task?.role ?? "worker";
  const eligible =
    profiles.value?.filter((p) => p.enabled && !p.archived && p.roles.includes(role)) ?? [];
  const profile = eligible.find((p) => p.id === target);
  const pending = records.value?.some(
    (h) => h.taskId === Number(taskId) && ["waiting", "ready", "executing"].includes(h.state)
  );
  return (
    <details className="asf-handoffs" aria-label="Agent handoffs">
      <summary>Agent assignment and handoff history</summary>
      {(records.error || error) && <p role="alert">{error || records.error}</p>}
      <p>
        A running Worker finishes its current process. ASF captures its changes before starting a
        replacement attempt. Review and integration remain required. Immediate process interruption
        is unavailable here.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (!profile || !task) return;
          setError("");
          setBusy(true);
          void requestHandoff(projectId, runId, task.id, profile, reason, key)
            .then(() => {
              setKey(crypto.randomUUID());
              return refreshData(projectId, `runs/${runId}/agent-handoffs`);
            })
            .catch((e) => setError(String(e)))
            .finally(() => setBusy(false));
        }}
      >
        <Select
          label="Task to reassign"
          value={taskId}
          onChange={(e) => {
            setTaskId(e.target.value);
            setTarget("");
            setKey(crypto.randomUUID());
          }}
        >
          <option value="">Select task</option>
          {tasks
            .filter((t) => ["pending", "ready", "running"].includes(t.state))
            .map((t) => (
              <option key={t.id} value={t.id}>
                #{t.id} · {t.title} · {t.state}
              </option>
            ))}
        </Select>
        {task && (
          <p>
            Current assignment:{" "}
            {detail.value?.attempts.filter((a) => a.taskId === task.id).at(-1)?.agent ??
              (task.state === "running" ? "Awaiting recorded agent data" : "Not dispatched")}
            . State: {task.state}. Backend verifies the current boundary.
          </p>
        )}
        <Select
          label="Replacement agent"
          value={target}
          onChange={(e) => {
            setTarget(e.target.value);
            setKey(crypto.randomUUID());
          }}
        >
          <option value="">Select profile</option>
          {eligible.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} · {p.providerAdapterId} · {p.modelId ?? "Provider default"} · r{p.revision}
            </option>
          ))}
        </Select>
        <p>
          Role eligibility is shown here. Rust revalidates permissions, executable availability,
          profile revision and task ownership before accepting.
        </p>
        <Textarea
          label="Handoff reason"
          value={reason}
          maxLength={2000}
          onChange={(e) => {
            setReason(e.target.value);
            setKey(crypto.randomUUID());
          }}
          required
          rows={2}
        />
        <Button
          type="submit"
          disabled={
            !task ||
            !["pending", "ready", "running"].includes(task.state) ||
            !profile ||
            !reason.trim() ||
            pending
          }
          loading={busy}
        >
          Request safe handoff
        </Button>
        {pending && <p role="status">A handoff is already pending for this task.</p>}
      </form>
      {records.value?.length === 0 && <p>No recorded handoffs.</p>}
      {records.value?.map((h) => (
        <section className="asf-handoff-record" key={h.id} aria-label={`Handoff ${h.id}`}>
          <strong>
            Task #{h.taskId} · {h.state}
          </strong>
          <p>
            {h.fromAgent ?? "Before dispatch"} → {h.target.name} ·{" "}
            {h.target.modelId ?? "Provider default"} · revision {h.target.revision}
          </p>
          <p>Boundary: {h.boundary.replaceAll("_", " ")}</p>
          <p>{h.reason}</p>
          {h.fromSessionId && (
            <Button
              onClick={() => dock.open("console", { projectId, resourceId: h.fromSessionId! })}
            >
              Original session #{h.fromSessionId}
            </Button>
          )}
          {h.replacementSessionId && (
            <Button
              onClick={() =>
                dock.open("console", { projectId, resourceId: h.replacementSessionId! })
              }
            >
              Replacement session #{h.replacementSessionId}
            </Button>
          )}
          <details>
            <summary>Preserved evidence</summary>
            <pre tabIndex={0}>{JSON.stringify(h.evidence, null, 2)}</pre>
          </details>
        </section>
      ))}
    </details>
  );
}

export function HandoffControls(props: { projectId: string; runId: number; dock: DockSession }) {
  return /^p_[0-9a-f]{32}$/.test(props.projectId) &&
    Number.isSafeInteger(props.runId) &&
    props.runId > 0 ? (
    <HandoffForm key={`${props.projectId}:${props.runId}`} {...props} />
  ) : (
    <p role="alert">Agent handoff requires a valid project and mission identity.</p>
  );
}
