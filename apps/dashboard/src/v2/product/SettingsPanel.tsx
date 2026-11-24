import { lazy, Suspense, useLayoutEffect, useRef, useState } from "react";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
import type { WorkspaceView, Preset } from "../workspace/production/model";
import { legacyUrl } from "../workspace/production/model";
import { Button, Input, Select, Checkbox } from "../design-system/primitives/Controls";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import { useData } from "../workspace/production/data";
import {
  defaultPreferences,
  savePreferences,
  usePreferences,
  type PreferencesEnvelope,
  type ApplicationPreferences,
} from "./preferences";
import { useMemoryData } from "../memory/client";
import type { Settings as MemoryPolicy } from "../memory/types";
import "./product.css";

const MemorySettings = lazy(() =>
  import("../memory/MemoryPanel").then((m) => ({ default: m.MemorySettings }))
);
const IdeSettings = lazy(() =>
  import("../ide/IdeSettings").then((m) => ({ default: m.IdeSettings }))
);
const settingsSections = [
  {
    id: "general",
    title: "General",
    scope: "Application",
    description: "Default layout and local application preferences",
  },
  {
    id: "appearance",
    title: "Appearance",
    scope: "Application",
    description: "Theme, light, dark, system and interface density",
  },
  {
    id: "workspace",
    title: "Workspace",
    scope: "Workspace",
    description: "Layouts, sessions, restoration and safe manual resume",
  },
  {
    id: "editor",
    title: "Editor",
    scope: "Application",
    description: "Font size, indentation tab size and word wrapping",
  },
  {
    id: "terminal",
    title: "Terminal and IDE tools",
    scope: "Project",
    description: "Approved shell, language server and debugger profiles",
  },
  {
    id: "agents",
    title: "Agents",
    scope: "Project",
    description: "Profiles, instructions, roles, permissions and capacity",
  },
  {
    id: "providers",
    title: "Providers and Models",
    scope: "Agent profile",
    description: "Configured adapters, model selection and availability",
  },
  {
    id: "missions",
    title: "Missions and Execution",
    scope: "Project",
    description: "Scheduler limits, routing and integration checks",
  },
  {
    id: "permissions",
    title: "Permissions and Security",
    scope: "Project",
    description: "Effective policy ceilings, authorizations and MCP grants",
  },
  {
    id: "extensions",
    title: "Extensions",
    scope: "Project",
    description: "Installed packages, trust and permissions",
  },
  {
    id: "memory",
    title: "Memory",
    scope: "Project",
    description: "Retrieval, extraction, source provenance and storage policy",
  },
  {
    id: "sourceControl",
    title: "Source Control",
    scope: "Project",
    description: "Repository, worktree ownership, index and local commit trust",
  },
  {
    id: "github",
    title: "GitHub",
    scope: "Project",
    description: "Remote identity, delivery policy and explicit publication authorization",
  },
  {
    id: "notifications",
    title: "Notifications",
    scope: "Application",
    description: "Durable events, brief feedback and optional browser notifications",
  },
  {
    id: "advanced",
    title: "Advanced / Diagnostics",
    scope: "Project",
    description: "Authoritative TOML configuration and service diagnostics",
  },
] as const;
export function SettingsPanel({
  model,
  dock,
  view,
}: {
  model: WorkspaceModel;
  dock?: DockSession;
  view?: WorkspaceView;
}) {
  const preferences = usePreferences();
  const content = useRef<HTMLDivElement>(null);
  const restoreEditFocus = useRef(false);
  const [editing, setEditing] = useState<PreferencesEnvelope>();
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const [query, setQuery] = useState("");
  const [section, setSection] = useState(view?.state.settingsSection ?? "general");
  const projectId = view?.projectId ?? dock?.session.projectId;
  const envelope = editing ?? preferences.value;
  const draft = envelope?.preferences ?? defaultPreferences;
  const modify = (value: Partial<ApplicationPreferences>) => {
    if (!envelope) return;
    setEditing({ ...envelope, preferences: { ...draft, ...value } });
    setNotice("");
  };
  useLayoutEffect(() => {
    if (busy || !restoreEditFocus.current) return;
    restoreEditFocus.current = false;
    if (document.activeElement === document.body) {
      content.current
        ?.querySelector<HTMLElement>(
          "input:not(:disabled),select:not(:disabled),button:not(:disabled),a[href]"
        )
        ?.focus();
    }
  }, [busy, editing]);
  const save = async () => {
    if (!editing) return;
    restoreEditFocus.current = content.current?.contains(document.activeElement) ?? false;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await savePreferences(editing);
      setEditing(undefined);
      setNotice("Application preferences saved by the server.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const selected = settingsSections.find((s) => s.id === section) ?? settingsSections[0];
  const select = (id: string) => {
    setSection(id);
    if (view) model.updateView(view.sessionId, view.id, { settingsSection: id });
    requestAnimationFrame(() =>
      content.current?.querySelector<HTMLElement>("input,select,button,a")?.focus()
    );
  };
  return (
    <section className="product-settings" aria-label="Settings">
      <header>
        <Input
          label="Search settings"
          type="search"
          placeholder="Search names and descriptions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </header>
      <div className="product-settings-body">
        <nav aria-label="Settings categories">
          {settingsSections
            .filter((s) =>
              `${s.title} ${s.description} ${s.scope}`.toLowerCase().includes(query.toLowerCase())
            )
            .map((s) => (
              <Button
                key={s.id}
                tone="quiet"
                aria-current={section === s.id ? "page" : undefined}
                onClick={() => select(s.id)}
              >
                <span>
                  {s.title}
                  <small>{s.scope}</small>
                </span>
              </Button>
            ))}
          {!settingsSections.some((s) =>
            `${s.title} ${s.description} ${s.scope}`.toLowerCase().includes(query.toLowerCase())
          ) ? (
            <p>No matching settings.</p>
          ) : null}
        </nav>
        <div className="product-settings-content" data-settings-content ref={content}>
          <h2>{selected.title}</h2>
          <p>
            {selected.scope} · {selected.description}
          </p>
          {preferences.error ? (
            <InlineAlert tone="danger" title="Preferences unavailable">
              {preferences.error}
              <Button
                onClick={() =>
                  void import("../workspace/production/data").then((m) =>
                    m.refreshData("", "/application/preferences")
                  )
                }
              >
                Reload preferences
              </Button>
            </InlineAlert>
          ) : null}
          {error ? (
            <InlineAlert tone="danger" title="Preferences were not saved">
              {error} Your edits are retained.
              <Button
                onClick={() => {
                  setEditing(undefined);
                  setError("");
                  void import("../workspace/production/data").then((m) =>
                    m.refreshData("", "/application/preferences")
                  );
                }}
              >
                Discard edits and reload current preferences
              </Button>
            </InlineAlert>
          ) : null}
          <div role="status">{notice}</div>
          {!envelope && !preferences.error ? (
            <LoadingIndicator label="Loading application preferences" />
          ) : null}
          {selected.id === "appearance" ? (
            <>
              <Select
                label="Application theme"
                value={draft.theme}
                disabled={!envelope || busy}
                onChange={(e) =>
                  modify({ theme: e.target.value as ApplicationPreferences["theme"] })
                }
              >
                <option value="system">System</option>
                <option value="dark">Dark</option>
                <option value="light">Light</option>
              </Select>
              <Select
                label="Interface density"
                value={draft.density}
                disabled={!envelope || busy}
                onChange={(e) =>
                  modify({ density: e.target.value as ApplicationPreferences["density"] })
                }
              >
                <option value="comfortable">Comfortable</option>
                <option value="compact">Compact</option>
              </Select>
            </>
          ) : selected.id === "editor" ? (
            <>
              <Input
                label="Editor font size"
                type="number"
                min={11}
                max={24}
                value={draft.editor_font_size}
                disabled={!envelope || busy}
                onChange={(e) => modify({ editor_font_size: Number(e.target.value) })}
              />
              <Input
                label="Indentation spaces"
                type="number"
                min={1}
                max={8}
                value={draft.editor_tab_size}
                disabled={!envelope || busy}
                onChange={(e) => modify({ editor_tab_size: Number(e.target.value) })}
              />
              <Checkbox
                label="Wrap long editor lines"
                checked={draft.editor_word_wrap}
                disabled={!envelope || busy}
                onChange={(e) => modify({ editor_word_wrap: e.target.checked })}
              />
              <p>
                Applies to repository editors without replacing buffers, undo history or effective
                write permissions.
              </p>
            </>
          ) : selected.id === "general" ? (
            <>
              <Select
                label="Default layout for new sessions"
                value={draft.default_layout}
                disabled={!envelope || busy}
                onChange={(e) => modify({ default_layout: e.target.value as Preset })}
              >
                {["Focus", "Code", "Agents", "Review"].map((s) => (
                  <option key={s}>{s}</option>
                ))}
              </Select>
              <p>
                Existing sessions retain their saved layouts. V1 remains the default application
                route.
              </p>
            </>
          ) : selected.id === "notifications" ? (
            <>
              <Checkbox
                label="Show brief notification feedback"
                checked={draft.notification_toasts}
                disabled={!envelope || busy}
                onChange={(e) => modify({ notification_toasts: e.target.checked })}
              />
              <Checkbox
                label="Enable optional browser notifications"
                checked={draft.browser_notifications}
                disabled={!envelope || busy || typeof Notification === "undefined"}
                onChange={(e) => modify({ browser_notifications: e.target.checked })}
              />
              <p>
                Important events remain in the notification center. Browser messages contain a
                generic event title and project identifier, never source code or agent output.
              </p>
              {typeof Notification !== "undefined" ? (
                <>
                  <p>Browser permission: {Notification.permission}.</p>
                  <Button
                    disabled={!draft.browser_notifications || Notification.permission === "denied"}
                    onClick={() =>
                      void Notification.requestPermission().then((permission) =>
                        setNotice(`Browser notification permission: ${permission}.`)
                      )
                    }
                  >
                    Request browser permission
                  </Button>
                </>
              ) : (
                <p>Browser notifications are unavailable in this browser.</p>
              )}
            </>
          ) : selected.id === "workspace" ? (
            <>
              <p>
                Presentation tabs can be closed without stopping work. Explicit session closure and
                mission suspension use the lifecycle controls. Resume remains manual after
                checkpoint and policy validation.
              </p>
              <div className="product-actions">
                {["Focus", "Code", "Agents", "Review"].map((p) => (
                  <Button key={p} disabled={!dock} onClick={() => dock?.preset(p as Preset)}>
                    {p} layout
                  </Button>
                ))}
              </div>
            </>
          ) : projectId ? (
            <ProjectSettings
              key={`${projectId}:${selected.id}`}
              projectId={projectId}
              section={selected.id}
              dock={dock}
              model={model}
            />
          ) : (
            <p>
              Open a registered project to inspect or change project-scoped configuration.
              Application preferences remain available here.
            </p>
          )}
          {editing ? (
            <div className="product-actions">
              <Button tone="primary" loading={busy} onClick={() => void save()}>
                Save application preferences
              </Button>
              <Button
                disabled={busy}
                onClick={() => {
                  restoreEditFocus.current =
                    content.current?.contains(document.activeElement) ?? false;
                  setEditing(undefined);
                  setError("");
                }}
              >
                Cancel preference edits
              </Button>
            </div>
          ) : null}
          <p className="product-muted">
            These preferences apply across ASF workspaces. Project permissions and agent access are
            managed separately in their project settings.
          </p>
        </div>
      </div>
    </section>
  );
}
function ProjectSettings({
  projectId,
  section,
  dock,
  model,
}: {
  projectId: string;
  section: string;
  dock?: DockSession;
  model: WorkspaceModel;
}) {
  const config = useData<Record<string, unknown>>(projectId, "settings");
  const open = (
    kind: "agents" | "memory" | "sourceControl" | "extensions" | "diagnostics" | "workflows"
  ) => dock?.open(kind, { projectId });
  const target =
    section === "agents" || section === "providers" || section === "permissions"
      ? "agents"
      : section === "memory"
        ? "memory"
        : section === "extensions"
          ? "extensions"
          : section === "sourceControl" || section === "github"
            ? "sourceControl"
            : section === "missions"
              ? "workflows"
              : "diagnostics";
  return (
    <>
      <p>
        Project <code>{projectId}</code>. Existing project and role policies remain authoritative.
        Profile edits affect future eligible invocations; active execution snapshots stay immutable.
      </p>
      <Button disabled={!dock} onClick={() => open(target)}>
        Open {target === "sourceControl" ? "Source Control" : target} configuration
      </Button>
      {config.error ? (
        <InlineAlert tone="warning" title="Project configuration unavailable">
          {config.error}
        </InlineAlert>
      ) : null}
      {section === "permissions" || section === "missions" ? (
        <details open>
          <summary>
            {section === "permissions"
              ? "Configured project ceiling and role policy"
              : "Configured execution limits and checks"}
          </summary>
          {config.value ? (
            <pre tabIndex={0}>
              {JSON.stringify(
                section === "permissions"
                  ? { policies: config.value.policies, roles: config.value.roles }
                  : {
                      runtime: config.value.runtime,
                      scheduler: config.value.scheduler,
                      integration_check_count: config.value.integration_check_count,
                    },
                null,
                2
              )}
            </pre>
          ) : (
            <LoadingIndicator label="Loading authoritative project configuration" />
          )}
        </details>
      ) : null}
      {section === "memory" ? <MemoryConfiguration projectId={projectId} /> : null}
      {section === "terminal" ? (
        <Suspense fallback={<LoadingIndicator label="Loading IDE configuration" />}>
          <IdeSettings projectId={projectId} model={model} />
        </Suspense>
      ) : null}
      <a href={legacyUrl(projectId)} target="_blank" rel="noopener">
        Open the authoritative project configuration editor in V1
      </a>
      <p>
        TOML and advanced policy controls retain their existing validation and revision checks. Tool
        installation and process trust require their established confirmations. Browser presence
        timings require server restart and are not changed by a UI preference.
      </p>
    </>
  );
}

function MemoryConfiguration({ projectId }: { projectId: string }) {
  const [version, setVersion] = useState(0);
  const value = useMemoryData<MemoryPolicy>(projectId, "/settings", version);
  return value.value ? (
    <Suspense fallback={<LoadingIndicator label="Loading memory configuration" />}>
      <MemorySettings
        key={value.value.revision}
        projectId={projectId}
        settings={value.value}
        saved={() => setVersion((v) => v + 1)}
      />
    </Suspense>
  ) : value.error ? (
    <InlineAlert tone="danger" title="Memory configuration unavailable">
      {value.error}
    </InlineAlert>
  ) : (
    <LoadingIndicator label="Loading project memory policy" />
  );
}
