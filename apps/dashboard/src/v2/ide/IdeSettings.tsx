import { useState } from "react";
import { Button, Textarea, Select } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import { ideRequest } from "./api";
import { refreshIde, useIde } from "./hooks";
import type { ToolProfile, LaunchConfiguration } from "./types";
import type { WorkspaceModel } from "../workspace/production/controller";

const example = {
  id: "language-server",
  name: "Language server",
  kind: "lsp",
  transport: "stdio",
  executable: "",
  arguments: [],
  languages: ["typescript"],
  file_patterns: ["*.ts"],
  initialization_options: {},
  settings: {},
  timeout_ms: 15000,
  enabled: true,
};
export function IdeSettings({ projectId, model }: { projectId: string; model: WorkspaceModel }) {
  const state = useIde(projectId);
  const [mode, setMode] = useState("profile"),
    [text, setText] = useState(JSON.stringify(example, null, 2)),
    [confirm, setConfirm] = useState(false),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const submit = async () => {
    setConfirm(false);
    setBusy(true);
    setError("");
    try {
      const value: unknown = JSON.parse(text);
      await ideRequest(
        projectId,
        mode === "profile" ? "profiles" : "configurations",
        mode === "profile"
          ? { profile: value as ToolProfile, confirmed: true }
          : { configuration: value as LaunchConfiguration, confirmed: true }
      );
      await refreshIde(projectId);
      model.announce("IDE configuration approved for this project.");
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="ide-settings" aria-label="IDE process settings">
      <h3>IDE tools</h3>
      <p>
        Profiles authorize a specific executable and fixed arguments. Tools inherit your OS
        permissions. API environment values are filtered; tokens are not added.
      </p>
      {error && (
        <InlineAlert tone="danger" title="Configuration refused">
          {error}
        </InlineAlert>
      )}
      {state.error && (
        <InlineAlert tone="danger" title="IDE settings unavailable">
          {state.error}
        </InlineAlert>
      )}
      <ul className="ide-list">
        {state.value?.profiles.map((profile) => (
          <li key={profile.id}>
            <span>
              {profile.name} · {profile.kind}
            </span>
            <Button
              tone="quiet"
              onClick={() => {
                setMode("profile");
                setText(JSON.stringify(profile, null, 2));
              }}
            >
              Edit
            </Button>
            <Button
              tone="quiet"
              onClick={() =>
                void ideRequest(projectId, `profiles/${encodeURIComponent(profile.id)}`, {})
                  .then(() => refreshIde(projectId))
                  .catch((error) => setError(String(error)))
              }
            >
              Remove profile
            </Button>
          </li>
        ))}
      </ul>
      <div>
        <Select
          label="Configuration type"
          value={mode}
          onChange={(e) => {
            setMode(e.target.value);
            setText(
              JSON.stringify(
                e.target.value === "profile"
                  ? example
                  : {
                      id: "debug-main",
                      name: "Debug main",
                      profile_id: "debugpy",
                      root_id:
                        model.documents.all().find((doc) => doc.file.project_id === projectId)?.file
                          .root_id ?? "",
                      program: model.documents
                        .all()
                        .find((doc) => doc.file.project_id === projectId)?.file ?? {
                        project_id: projectId,
                        root_id: "",
                        path: "main.py",
                      },
                      arguments: { stopOnEntry: false },
                    },
                null,
                2
              )
            );
          }}
        >
          <option value="profile">Tool profile</option>
          <option value="launch">Debug launch configuration</option>
        </Select>
      </div>
      <div>
        <Textarea
          label="IDE configuration JSON"
          rows={14}
          spellCheck={false}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
      </div>
      <Button
        disabled={busy}
        onClick={() => {
          try {
            JSON.parse(text);
            setConfirm(true);
          } catch {
            setError("Enter valid JSON before approval.");
          }
        }}
      >
        Review and approve configuration
      </Button>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title="Approve IDE process configuration"
        description="This authorizes a local executable or project-code launch configuration. Review the path and arguments above. Launching a tool can read or change files with your OS permissions."
        footer={
          <>
            <Button tone="quiet" onClick={() => setConfirm(false)}>
              Cancel
            </Button>
            <Button onClick={() => void submit()}>Approve configuration</Button>
          </>
        }
      />
    </section>
  );
}
