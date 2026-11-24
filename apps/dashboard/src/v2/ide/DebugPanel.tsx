import { useEffect, useState, useSyncExternalStore } from "react";
import { Button, Input, Select } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import type { WorkspaceView } from "../workspace/production/model";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
import type { Variable } from "./debug";
import { useIde, refreshIde } from "./hooks";
import { fileKey } from "../files/types";

export function DebugPanel({
  view,
  model,
  dock,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  const api = useIde(view.projectId),
    manager = model.debug;
  useSyncExternalStore(manager.subscribe, manager.snapshot);
  const [configuration, setConfiguration] = useState(""),
    [selected, setSelected] = useState(""),
    [confirm, setConfirm] = useState<"launch" | "evaluate" | null>(null),
    [expression, setExpression] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [evaluation, setEvaluation] = useState("");
  useEffect(() => {
    for (const info of api.value?.debug_sessions ?? []) manager.adopt(view.projectId, info);
  }, [api.value, manager, view.projectId]);
  const sessions = [...manager.sessions]
    .filter(([key]) => key.startsWith(`${view.projectId}:`))
    .map(([, session]) => session);
  const session = sessions.find((s) => s.info.id === selected) ?? sessions.at(-1);
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const command = (name: string) =>
    run(async () => {
      if (!session) return;
      await manager.command(view.projectId, session.info.id, name, { threadId: session.threadId });
    });
  const inspect = async (variable: Variable) => {
    if (!session) return;
    const result = (await manager.command(view.projectId, session.info.id, "variables", {
      variablesReference: variable.variablesReference,
    })) as { variables: Variable[] };
    session.variables = result.variables;
    setEvaluation(`Expanded ${variable.name}`);
  };
  return (
    <div className="ide-debug-panel ws-scroll">
      <div className="files-toolbar" role="toolbar" aria-label="Debug controls">
        <Select
          label="Launch configuration"
          hideLabel
          value={configuration || api.value?.configurations[0]?.id || ""}
          onChange={(e) => setConfiguration(e.target.value)}
        >
          <option value="" disabled>
            Select configuration
          </option>
          {api.value?.configurations.map((config) => (
            <option key={config.id} value={config.id}>
              {config.name}
            </option>
          ))}
        </Select>
        <Button
          disabled={busy || !api.value?.configurations.length}
          onClick={() => setConfirm("launch")}
        >
          Start debugging
        </Button>
        <Button tone="quiet" onClick={() => dock.open("settings")}>
          Debug settings
        </Button>
      </div>
      {error && (
        <InlineAlert tone="danger" title="Debug operation failed">
          {error}
        </InlineAlert>
      )}
      {api.error && (
        <InlineAlert tone="danger" title="Debugger unavailable">
          {api.error}
        </InlineAlert>
      )}
      {sessions.length > 0 && (
        <Select
          label="Debug session"
          value={session?.info.id ?? ""}
          onChange={(e) => setSelected(e.target.value)}
        >
          {sessions.map((session) => (
            <option key={session.info.id} value={session.info.id}>
              {session.info.configuration_id} · {session.status}
            </option>
          ))}
        </Select>
      )}
      {session ? (
        <>
          <Select
            label="Debug thread"
            value={String(session.threadId ?? "")}
            onChange={(event) =>
              void run(() =>
                manager.selectThread(view.projectId, session, Number(event.target.value))
              )
            }
            disabled={!session.frames.length}
          >
            {session.threads.map((thread) => (
              <option key={thread.id} value={thread.id}>
                {thread.name}
              </option>
            ))}
          </Select>
          <div className="files-toolbar">
            <span role="status">{session.status}</span>
            <Button
              disabled={busy || session.stopped || !session.frames.length}
              onClick={() => void command("continue")}
            >
              Continue
            </Button>
            <Button
              disabled={busy || session.stopped || session.status !== "Running"}
              onClick={() => void command("pause")}
            >
              Pause
            </Button>
            <Button
              disabled={busy || session.stopped || !session.frames.length}
              onClick={() => void command("next")}
            >
              Step over
            </Button>
            <Button
              disabled={busy || session.stopped || !session.frames.length}
              onClick={() => void command("stepIn")}
            >
              Step into
            </Button>
            <Button
              disabled={busy || session.stopped || !session.frames.length}
              onClick={() => void command("stepOut")}
            >
              Step out
            </Button>
            <Button
              onClick={() =>
                void run(async () => {
                  await manager.stop(view.projectId, session.info.id);
                  await refreshIde(view.projectId);
                })
              }
            >
              Stop debugger
            </Button>
          </div>
          {session.error && (
            <InlineAlert tone="warning" title="Debugger event problem">
              {session.error}
            </InlineAlert>
          )}
          <h3>Call stack</h3>
          <ol className="ide-list">
            {session.frames.map((frame) => (
              <li key={frame.id}>
                <Button
                  tone="quiet"
                  onClick={() =>
                    void run(async () => {
                      await manager.selectFrame(view.projectId, session, frame);
                      if (frame.asfFile)
                        await dock.openFile(frame.asfFile, "within", {
                          line: frame.line,
                          column: frame.column,
                        });
                    })
                  }
                >
                  {frame.name} · {frame.asfFile?.path ?? "Source unavailable"}:{frame.line}
                </Button>
              </li>
            ))}
          </ol>
          <h3>Variables</h3>
          <dl className="ide-variables">
            {session.variables.slice(0, 200).map((variable, i) => (
              <div key={`${variable.name}:${i}`}>
                <dt>
                  {variable.scope && (
                    <small className="ide-variable-scope">{variable.scope} · </small>
                  )}
                  {variable.variablesReference > 0 ? (
                    <Button tone="quiet" onClick={() => void run(() => inspect(variable))}>
                      {variable.name}
                    </Button>
                  ) : (
                    variable.name
                  )}
                </dt>
                <dd title={variable.value}>{variable.value}</dd>
              </div>
            ))}
          </dl>
          <h3>Debug console</h3>
          <div className="files-toolbar">
            <Input
              label="Expression"
              value={expression}
              onChange={(e) => setExpression(e.target.value)}
            />
            <Button
              disabled={
                !expression.trim() ||
                !session.frames.length ||
                !session.info.capabilities.supportsEvaluateForHovers
              }
              onClick={() => setConfirm("evaluate")}
            >
              Evaluate expression
            </Button>
          </div>
          <pre className="ide-debug-output" tabIndex={0}>
            {evaluation}
            {evaluation ? "\n" : ""}
            {session.output || "No debugger output received."}
          </pre>
        </>
      ) : (
        <p>
          Configure an approved adapter and a launch profile. Opening this panel does not execute
          project code. Attach to arbitrary PIDs is disabled.
        </p>
      )}
      <h3>Breakpoints</h3>
      <ul className="ide-list">
        {manager.breakpoints
          .filter((point) => point.file.project_id === view.projectId)
          .map((point) => (
            <li key={`${fileKey(point.file)}:${point.line}`}>
              <Button
                tone="quiet"
                onClick={() => void dock.openFile(point.file, "within", { line: point.line })}
              >
                {point.file.path}:{point.line}
              </Button>
              <Input
                label={`Condition ${point.file.path}:${point.line}`}
                value={point.condition ?? ""}
                onChange={(e) => manager.condition(point, e.target.value)}
                placeholder="Optional condition"
              />
              <Button tone="quiet" onClick={() => manager.toggle(point.file, point.line)}>
                Remove breakpoint
              </Button>
            </li>
          ))}
      </ul>
      <Dialog
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title={confirm === "launch" ? "Launch project debugger" : "Evaluate debug expression"}
        description={
          confirm === "launch"
            ? "This executes the configured project program under the approved adapter with your OS permissions. Attach is disabled; external terminals and adapter-requested processes are refused."
            : "Expression evaluation can execute code or change the running program. Only evaluate an expression you trust."
        }
        footer={
          <>
            <Button tone="quiet" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                const action = confirm;
                setConfirm(null);
                void run(async () => {
                  if (action === "launch") {
                    const info = await manager.start(
                      view.projectId,
                      configuration || api.value?.configurations[0]?.id || ""
                    );
                    setSelected(info.id);
                    await refreshIde(view.projectId);
                  } else if (session) {
                    const result = await manager.command(
                      view.projectId,
                      session.info.id,
                      "evaluate",
                      { expression, frameId: session.activeFrameId, context: "repl" },
                      true
                    );
                    setEvaluation(JSON.stringify(result, null, 2));
                  }
                });
              }}
            >
              {confirm === "launch"
                ? "Launch approved configuration"
                : "Evaluate trusted expression"}
            </Button>
          </>
        }
      />
    </div>
  );
}
