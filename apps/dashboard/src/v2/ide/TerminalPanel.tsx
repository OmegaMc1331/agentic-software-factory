import { lazy, Suspense, useState } from "react";
import { Button, Input, Select } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import { useData } from "../workspace/production/data";
import type { WorkspaceView } from "../workspace/production/model";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
import type { RootCatalog } from "../files/types";
import { useIde, refreshIde } from "./hooks";
import { ideRequest } from "./api";
import type { TerminalInfo } from "./types";
const TerminalView = lazy(() => import("../workspace/production/TerminalView"));
export function DevelopmentTerminalPanel({
  view,
  dock,
  model,
}: {
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
}) {
  const state = useIde(view.projectId),
    roots = useData<RootCatalog>(view.projectId, "files/roots");
  const [profile, setProfile] = useState(""),
    [confirm, setConfirm] = useState<"start" | "stop" | null>(null),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [name, setName] = useState("");
  const terminal = state.value?.terminals.find((t) => t.id === view.agent);
  const profiles = state.value?.profiles.filter((p) => p.kind === "shell" && p.enabled) ?? [];
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await action();
      await refreshIde(view.projectId);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const start = () =>
    run(async () => {
      const root = roots.value?.roots.find((root) => !root.read_only);
      if (!root) throw new Error("No authorized writable root is available.");
      const created = await ideRequest<TerminalInfo>(view.projectId, "terminals/start", {
        root_id: root.id,
        profile_id: profile || profiles[0]?.id,
        confirmed: true,
        cols: 100,
        rows: 28,
      });
      dock.open("developmentTerminal", {
        agent: created.id,
        projectId: view.projectId,
        title: created.name,
      });
    });
  return (
    <div className="ide-terminal-panel">
      <div className="files-toolbar">
        <Select
          label="Shell profile"
          hideLabel
          value={profile || profiles[0]?.id || ""}
          onChange={(e) => setProfile(e.target.value)}
        >
          <option value="" disabled>
            Select approved shell
          </option>
          {profiles.map((profile) => (
            <option key={profile.id} value={profile.id}>
              {profile.name}
            </option>
          ))}
        </Select>
        <Button
          disabled={busy || !profiles.length || !roots.value?.roots.some((root) => !root.read_only)}
          onClick={() => setConfirm("start")}
        >
          New terminal
        </Button>
        <Button tone="quiet" onClick={() => dock.open("settings")}>
          Shell settings
        </Button>
        {terminal && (
          <>
            <Button
              tone="quiet"
              onClick={() =>
                dock.open(
                  "developmentTerminal",
                  { agent: terminal.id, projectId: view.projectId, title: terminal.name },
                  "below",
                  true
                )
              }
            >
              Split terminal view
            </Button>
            <Button
              tone="quiet"
              disabled={terminal.exit_code !== null}
              onClick={() => setConfirm("stop")}
            >
              Terminate shell
            </Button>
            <Button
              tone="quiet"
              disabled={terminal.exit_code === null}
              onClick={() => {
                setProfile(terminal.profile_id);
                setConfirm("start");
              }}
            >
              Restart terminal
            </Button>
            <span>
              {terminal.exit_code === null ? `PID ${terminal.pid}` : `Exited ${terminal.exit_code}`}
            </span>
          </>
        )}
      </div>
      {error && (
        <InlineAlert title="Terminal operation failed" tone="danger">
          {error}
        </InlineAlert>
      )}
      <div className="ide-terminal-tabs" role="toolbar" aria-label="Project terminals">
        {state.value?.terminals.map((terminal) => (
          <Button
            key={terminal.id}
            tone={terminal.id === view.agent ? "primary" : "quiet"}
            onClick={() =>
              dock.open("developmentTerminal", {
                agent: terminal.id,
                projectId: view.projectId,
                title: terminal.name,
              })
            }
          >
            {terminal.name}
          </Button>
        ))}
      </div>
      {roots.loading && <LoadingIndicator label="Loading authorized terminal roots" />}
      {roots.error && (
        <InlineAlert title="Terminal roots unavailable" tone="danger">
          {roots.error}
        </InlineAlert>
      )}
      {terminal ? (
        <>
          <details>
            <summary>Rename terminal</summary>
            <div className="files-toolbar">
              <Input
                label="Terminal name"
                value={name}
                placeholder={terminal.name}
                onChange={(e) => setName(e.target.value)}
              />
              <Button
                disabled={!name.trim()}
                onClick={() =>
                  void run(async () => {
                    await ideRequest(view.projectId, `terminals/${terminal.id}/rename`, { name });
                    model.announce("Terminal renamed.");
                  })
                }
              >
                Rename terminal
              </Button>
            </div>
          </details>
          <Suspense fallback={<LoadingIndicator label="Loading terminal" />}>
            <TerminalView
              key={`${view.projectId}:${terminal.id}`}
              projectId={view.projectId}
              development={terminal}
            />
          </Suspense>
        </>
      ) : (
        <div className="ws-scroll">
          <p>
            Start an approved local shell or reconnect to a listed terminal. A development shell has
            your OS account permissions. Its starting directory is not a sandbox.
          </p>
          {view.agent && (
            <p>
              The saved terminal process is no longer available after backend restart. Start a new
              terminal explicitly.
            </p>
          )}
        </div>
      )}
      <Dialog
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title={confirm === "start" ? "Start development shell" : "Terminate development shell"}
        description={
          confirm === "start"
            ? "The approved shell runs in this project with your OS account permissions. It may access other files and networks. No command is executed merely by opening this panel."
            : "The shell and supervised descendants will be terminated. Running commands may be interrupted. Closing a view alone does not do this."
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
                void (action === "start"
                  ? start()
                  : run(async () => {
                      if (terminal)
                        await ideRequest(view.projectId, `terminals/${terminal.id}/stop`, {});
                    }));
              }}
            >
              {confirm === "start" ? "Start approved shell" : "Terminate shell"}
            </Button>
          </>
        }
      />
    </div>
  );
}
