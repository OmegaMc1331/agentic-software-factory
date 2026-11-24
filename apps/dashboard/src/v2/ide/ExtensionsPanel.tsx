import { useState } from "react";
import { Button, Input, Textarea } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession } from "../workspace/production/controller";
import { ideRequest } from "./api";
import { useIde, refreshIde } from "./hooks";
import type { ExtensionManifest } from "./types";
export function ExtensionsPanel({ view, dock }: { view: WorkspaceView; dock: DockSession }) {
  const state = useIde(view.projectId);
  const [source, setSource] = useState(""),
    [error, setError] = useState(""),
    [install, setInstall] = useState<ExtensionManifest | null>(null),
    [remove, setRemove] = useState<string | null>(null),
    [busy, setBusy] = useState(false);
  const action = async (run: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await run();
      await refreshIde(view.projectId);
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="ws-scroll ide-extensions">
      <h2>Extensions</h2>
      <p>
        ASF declarative API 1. Packages contribute editor behavior, commands and workspace views.
        Arbitrary JavaScript, install scripts and VS Code extensions are not supported.
      </p>
      {(error || state.error) && (
        <InlineAlert title="Extension operation failed" tone="danger">
          {error || state.error}
        </InlineAlert>
      )}
      <ul className="ide-list">
        {state.value?.extensions.map((extension) => (
          <li className="ide-extension-row" key={extension.manifest.id}>
            <div>
              <strong>{extension.manifest.name}</strong>
              <span>
                {" "}
                {extension.manifest.version} · {extension.status}
              </span>
              <p>{extension.manifest.description}</p>
              <small>
                {extension.manifest.id} · {extension.manifest.publisher}
              </small>
              <details>
                <summary>Permissions and provenance</summary>
                <p>{extension.manifest.permissions.join(", ") || "No capabilities"}</p>
                <p>{extension.provenance}</p>
                <code>{extension.sha256}</code>
              </details>
              {extension.error && <p role="alert">{extension.error}</p>}
              {Object.entries(
                extension.preferences ?? extension.manifest.contributions.settings ?? {}
              ).map(([key, value]) => (
                <Input
                  key={`${extension.manifest.id}:${key}:${value}`}
                  label={`${extension.manifest.name} · ${key}`}
                  defaultValue={value}
                  onBlur={(event) => {
                    if (event.target.value !== value)
                      void action(async () => {
                        await ideRequest(
                          view.projectId,
                          `extensions/${extension.manifest.id}/settings`,
                          { settings: { ...extension.preferences, [key]: event.target.value } }
                        );
                      });
                  }}
                />
              ))}
            </div>
            <div className="files-toolbar">
              {extension.enabled &&
                extension.status === "active" &&
                extension.manifest.contributions.commands
                  ?.filter((command) => command.action === "open-view")
                  .map((command) => (
                    <Button
                      key={command.id}
                      tone="quiet"
                      onClick={() =>
                        dock.open("extensionView", {
                          agent: `${extension.manifest.id}/${command.target}`,
                          title: command.title,
                        })
                      }
                    >
                      {command.title}
                    </Button>
                  ))}
              <Button
                disabled={busy}
                onClick={() =>
                  void action(async () => {
                    await ideRequest(
                      view.projectId,
                      `extensions/${extension.manifest.id}/${extension.enabled ? "disable" : "enable"}`,
                      {}
                    );
                  })
                }
              >
                {extension.enabled ? "Disable" : "Enable"}
              </Button>
              <Button tone="quiet" disabled={busy} onClick={() => setRemove(extension.manifest.id)}>
                Uninstall
              </Button>
              {extension.enabled &&
                extension.status === "active" &&
                extension.manifest.contributions.views?.map((panel) => (
                  <Button
                    key={panel.id}
                    tone="quiet"
                    onClick={() =>
                      dock.open("extensionView", {
                        agent: `${extension.manifest.id}/${panel.id}`,
                        title: panel.title,
                      })
                    }
                  >
                    {panel.title}
                  </Button>
                ))}
            </div>
          </li>
        ))}
      </ul>
      <details>
        <summary>Install trusted local package</summary>
        <p>
          Import a JSON package. Only validated declarative contributions are accepted; no archive
          or executable payload is extracted.
        </p>
        <input
          aria-label="Extension package file"
          type="file"
          accept="application/json,.json"
          onChange={(e) => {
            const file = e.target.files?.[0];
            if (!file) return;
            if (file.size > 131072) {
              setError("Package exceeds 128 KiB.");
              return;
            }
            void file
              .text()
              .then(setSource)
              .catch((error) => setError(String(error)));
          }}
        />
        <Textarea
          label="Extension package JSON"
          rows={10}
          value={source}
          onChange={(e) => setSource(e.target.value)}
        />
        <Button
          disabled={busy || !source.trim()}
          onClick={() => {
            try {
              setInstall(JSON.parse(source.replace(/^\uFEFF/, "")) as ExtensionManifest);
            } catch {
              setError("Invalid package JSON.");
            }
          }}
        >
          Review package
        </Button>
      </details>
      <Dialog
        open={install !== null}
        onClose={() => setInstall(null)}
        title="Install declarative extension"
        description={`Review publisher, version and permissions before import. ${install?.name ?? ""} ${install?.version ?? ""} · ${install?.permissions?.join(", ") ?? ""}. Installation does not activate it.`}
        footer={
          <>
            <Button tone="quiet" onClick={() => setInstall(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                const manifest = install;
                setInstall(null);
                void action(async () => {
                  await ideRequest(view.projectId, "extensions/install", {
                    manifest,
                    trusted: true,
                  });
                  setSource("");
                });
              }}
            >
              Trust and install
            </Button>
          </>
        }
      />
      <Dialog
        open={remove !== null}
        onClose={() => setRemove(null)}
        title="Uninstall extension"
        description="Its contributions will be removed. Open extension views become an unavailable state. Project files are not deleted."
        footer={
          <>
            <Button tone="quiet" onClick={() => setRemove(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                const id = remove;
                setRemove(null);
                void action(async () => {
                  await ideRequest(view.projectId, `extensions/${id}/uninstall`, {});
                });
              }}
            >
              Uninstall extension
            </Button>
          </>
        }
      />
    </div>
  );
}
export function ExtensionView({ view }: { view: WorkspaceView }) {
  const state = useIde(view.projectId);
  const [extensionId, panelId] = (view.agent ?? "").split("/");
  const extension = state.value?.extensions.find(
    (e) => e.manifest.id === extensionId && e.enabled && e.status === "active"
  );
  const panel = extension?.manifest.contributions.views?.find((panel) => panel.id === panelId);
  return (
    <div className="ws-scroll">
      <h2>{panel?.title ?? "Extension view unavailable"}</h2>
      <p className="ide-extension-text">
        {panel?.text ??
          "Enable or reinstall the contributing extension. This view does not execute extension code."}
      </p>
    </div>
  );
}
