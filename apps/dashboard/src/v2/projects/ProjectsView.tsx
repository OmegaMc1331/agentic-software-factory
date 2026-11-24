import { useCallback, useEffect, useState, type FormEvent } from "react";
import { Button, Checkbox, Input, Select } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { CompactStatus, InlineAlert } from "../design-system/components/Application";
import { useTheme, type ThemePreference } from "../design-system/themes/theme";
import {
  applicationInfo,
  listProjects,
  projectMutation,
  projectPath,
  type ApplicationInfo,
  type Project,
} from "./projectApi";
import { useProjectSnapshot } from "./useProjectSnapshot";
import { savePreferences, usePreferences } from "../product/preferences";
import { FactoryApiError } from "../../api";

type Action = "register" | "create" | "clone" | "rename" | "relink" | "initialize" | "remove";
const titles: Record<Action, string> = {
  register: "Register repository",
  create: "Create project",
  clone: "Clone repository",
  rename: "Rename project",
  relink: "Relink project",
  initialize: "Initialize Factory",
  remove: "Remove from registry",
};
const storageKey = "asf.v2.projects.selection.v1";
function readSelection() {
  try {
    return localStorage.getItem(storageKey) ?? "";
  } catch {
    return "";
  }
}

export function ProjectsView() {
  const theme = useTheme();
  const preferences = usePreferences();
  const [themeBusy, setThemeBusy] = useState(false);
  const [projects, setProjects] = useState<Project[]>([]);
  const [info, setInfo] = useState<ApplicationInfo>();
  const [active, setActive] = useState(readSelection);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [operation, setOperation] = useState<{ action: Action; target?: Project }>();
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [remote, setRemote] = useState("");
  const [trusted, setTrusted] = useState(false);
  const [network, setNetwork] = useState(false);
  const [initializeClone, setInitializeClone] = useState(true);
  const [identityChange, setIdentityChange] = useState(false);
  useEffect(() => {
    const saved = preferences.value;
    if (saved) document.documentElement.dataset.density = saved.preferences.density;
    if (saved && saved.revision > 0 && theme.preference !== saved.preferences.theme)
      theme.setPreference(saved.preferences.theme);
  }, [preferences.value, theme]);
  const selected = projects.find(
    (project) => project.project_id === active && project.lifecycle === "open"
  );
  const { snapshot, error: snapshotError } = useProjectSnapshot(
    selected?.project_id ?? "",
    revision
  );
  const refresh = useCallback(async (signal?: AbortSignal) => {
    const [next, application] = await Promise.all([listProjects(signal), applicationInfo(signal)]);
    if (signal?.aborted) return;
    setProjects(next);
    setInfo(application);
    setLoaded(true);
    setRevision((value) => value + 1);
    setActive((current) =>
      next.some((project) => project.project_id === current && project.lifecycle === "open")
        ? current
        : (next.find((project) => project.lifecycle === "open")?.project_id ?? "")
    );
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal).catch((failure: Error) => {
      if (!controller.signal.aborted) {
        setError(failure.message);
        setLoaded(true);
      }
    });
    return () => controller.abort();
  }, [refresh]);
  const select = (id: string) => {
    setActive(id);
    try {
      localStorage.setItem(storageKey, id);
    } catch {
      setError("Project selected for this session. Browser storage is unavailable.");
    }
  };
  const mutate = async (work: () => Promise<unknown>, after?: () => void) => {
    setBusy(true);
    setError("");
    try {
      await work();
      await refresh();
      after?.();
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "Project operation failed. Inspect the registry before retrying."
      );
    } finally {
      setBusy(false);
    }
  };
  const begin = (action: Action, target?: Project) => {
    setPath(target?.canonical_root ?? "");
    setName(target?.display_name ?? "");
    setRemote("");
    setTrusted(false);
    setNetwork(false);
    setInitializeClone(true);
    setIdentityChange(false);
    setOperation({ action, target });
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!operation || busy) return;
    const { action, target } = operation;
    const body =
      action === "register"
        ? { path, display_name: name || undefined, trust_project: trusted }
        : action === "create"
          ? { destination: path, display_name: name, trust_project: trusted }
          : action === "clone"
            ? {
                destination: path,
                display_name: name,
                remote,
                initialize: initializeClone,
                allow_network: network,
                trust_project: trusted,
              }
            : action === "relink"
              ? { path, accept_identity_change: identityChange, trust_project: trusted }
              : action === "rename"
                ? { display_name: name }
                : action === "initialize"
                  ? { trust_project: trusted }
                  : undefined;
    const endpoint = target
      ? projectPath(target.project_id, action === "rename" || action === "remove" ? "" : action)
      : `/projects/${action}`;
    void mutate(
      () =>
        projectMutation(
          endpoint,
          action === "rename" ? "PUT" : action === "remove" ? "DELETE" : "POST",
          body
        ),
      () => setOperation(undefined)
    );
  };
  const action = operation?.action;
  const closeWhenIdle = async (projectId: string) => {
    for (let attempt = 0; ; attempt++) {
      try {
        return await projectMutation(projectPath(projectId, "close"), "POST", {});
      } catch (failure) {
        if (
          !(failure instanceof FactoryApiError) ||
          failure.status !== 409 ||
          !failure.message.includes("project owns active work or live requests") ||
          attempt >= 5
        )
          throw failure;
        await new Promise((resolve) => setTimeout(resolve, 150));
      }
    }
  };
  const needsTrust =
    !!action && ["register", "create", "clone", "relink", "initialize"].includes(action);
  return (
    <div className="projects-app">
      <header className="projects-top">
        <a href="/" className="projects-brand">
          ASF
        </a>
        <span>Projects</span>
        <span className="projects-spacer" />
        <a href="/">V1 workspace</a>
        <a href="/workspace.html">V2 workspace</a>
        <Select
          label="Theme"
          hideLabel
          value={theme.preference}
          disabled={themeBusy || !preferences.value || !!preferences.error}
          onChange={(event) => {
            if (!preferences.value) return;
            setThemeBusy(true);
            void savePreferences({
              ...preferences.value,
              preferences: {
                ...preferences.value.preferences,
                theme: event.target.value as ThemePreference,
              },
            })
              .catch((failure) => setError(String(failure)))
              .finally(() => setThemeBusy(false));
          }}
        >
          <option value="system">System</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </Select>
      </header>
      <main className="projects-main">
        {preferences.error ? (
          <InlineAlert tone="danger" title="Appearance settings unavailable">
            {preferences.error}
          </InlineAlert>
        ) : null}
        <div className="projects-heading">
          <div>
            <h1>Local projects</h1>
            <p>One server. Independent repositories, runtimes and policies.</p>
          </div>
          <div className="projects-actions">
            <Button onClick={() => begin("register")} disabled={busy}>
              Register
            </Button>
            <Button onClick={() => begin("clone")} disabled={busy}>
              Clone
            </Button>
            <Button tone="primary" onClick={() => begin("create")} disabled={busy}>
              Create project
            </Button>
          </div>
        </div>
        {error && (
          <InlineAlert
            tone="danger"
            title="Project operation needs attention"
            onDismiss={() => setError("")}
          >
            {error}
          </InlineAlert>
        )}
        <div className="projects-policy">
          <span>Authorized directories</span>
          {info?.authorized_roots.map((root) => (
            <code key={root}>{root}</code>
          ))}
          <span>
            Additional directories require <code>factory start --allow-project-root</code> at server
            startup.
          </span>
        </div>
        <div className="projects-columns">
          <section aria-label="Registered projects" className="projects-list">
            <div className="projects-section-title">
              <h2>Registry</h2>
              <Button tone="quiet" disabled={busy} onClick={() => void mutate(() => refresh())}>
                Refresh
              </Button>
            </div>
            {!loaded && <p role="status">Loading registry...</p>}
            {loaded && !projects.length && (
              <p>
                No projects registered. Register a repository or create one in an authorized
                directory.
              </p>
            )}
            {projects.map((project) => (
              <article
                className="projects-row"
                key={project.project_id}
                data-selected={selected?.project_id === project.project_id || undefined}
              >
                <div className="projects-row-title">
                  <button
                    className="projects-select"
                    disabled={project.lifecycle !== "open"}
                    onClick={() => select(project.project_id)}
                    aria-pressed={selected?.project_id === project.project_id}
                  >
                    {project.display_name}
                  </button>
                  <CompactStatus
                    status={
                      project.availability === "unavailable"
                        ? "attention"
                        : project.lifecycle === "open"
                          ? "running"
                          : "idle"
                    }
                    label={
                      project.availability === "unavailable"
                        ? "Unavailable"
                        : project.availability === "not_initialized"
                          ? "Needs initialization"
                          : project.lifecycle
                    }
                  />
                </div>
                <code className="projects-path">{project.canonical_root}</code>
                {project.project_id === info?.default_project_id && (
                  <span className="projects-default">V1 default - fixed at server startup</span>
                )}
                {project.problem && <p className="projects-problem">{project.problem}</p>}
                <div className="projects-actions">
                  {project.lifecycle === "open" ? (
                    <>
                      <Button disabled={busy} onClick={() => select(project.project_id)}>
                        Select
                      </Button>
                      <Button
                        tone="quiet"
                        disabled={busy}
                        onClick={() => void mutate(() => closeWhenIdle(project.project_id))}
                      >
                        Close when idle
                      </Button>
                    </>
                  ) : (
                    <>
                      <Button
                        disabled={busy || project.availability === "not_initialized"}
                        onClick={() =>
                          void mutate(async () => {
                            await projectMutation(
                              projectPath(project.project_id, "open"),
                              "POST",
                              {}
                            );
                            select(project.project_id);
                          })
                        }
                      >
                        Open
                      </Button>
                      {project.availability === "not_initialized" && (
                        <Button disabled={busy} onClick={() => begin("initialize", project)}>
                          Initialize
                        </Button>
                      )}
                      <Button tone="quiet" disabled={busy} onClick={() => begin("relink", project)}>
                        Relink
                      </Button>
                      <Button tone="quiet" disabled={busy} onClick={() => begin("remove", project)}>
                        Remove
                      </Button>
                    </>
                  )}
                  <Button tone="quiet" disabled={busy} onClick={() => begin("rename", project)}>
                    Rename
                  </Button>
                </div>
              </article>
            ))}
          </section>
          <section aria-label="Selected project" className="projects-detail">
            <div className="projects-section-title">
              <h2>{selected?.display_name ?? "Project context"}</h2>
              {selected && <CompactStatus status="running" label="Open" />}
            </div>
            {!selected ? (
              <p>Select an open project to inspect its workflows and configuration.</p>
            ) : (
              <>
                <code className="projects-id">{selected.project_id}</code>
                {snapshotError && (
                  <InlineAlert tone="danger" title="Project context could not load">
                    {snapshotError}
                  </InlineAlert>
                )}
                {!snapshot && !snapshotError && <p role="status">Loading project context...</p>}
                {snapshot && (
                  <>
                    <h3>Workflows</h3>
                    {!snapshot.runs.length && <p>No workflows in this project.</p>}
                    <ul className="projects-runs">
                      {snapshot.runs.slice(0, 50).map((run) => (
                        <li key={run.id}>
                          <code>#{run.id}</code>
                          <span>{run.objective}</span>
                          <span>{run.status}</span>
                        </li>
                      ))}
                    </ul>
                    {snapshot.hasMore && <p>Showing the first 50 workflows.</p>}
                    <h3>Project configuration</h3>
                    <p>
                      Agents: {Object.keys(snapshot.config.agents).join(", ") || "none configured"}
                    </p>
                    <pre aria-label="Project role assignments">
                      {JSON.stringify(snapshot.config.role_assignments, null, 2)}
                    </pre>
                    <p>
                      MCP servers:{" "}
                      {Object.keys(snapshot.config.mcp ?? {}).join(", ") || "none configured"}
                    </p>
                  </>
                )}
              </>
            )}
          </section>
        </div>
      </main>
      <Dialog
        open={!!operation}
        onClose={() => {
          if (!busy) setOperation(undefined);
        }}
        title={action ? titles[action] : "Project"}
        description={
          action === "remove"
            ? "Removes the registry entry only. Repository files and .factory history are preserved."
            : "Backend paths are validated against the server's authorized directories. A browser path entry does not grant filesystem access."
        }
      >
        <form className="projects-form" onSubmit={submit}>
          {action && ["register", "create", "clone", "relink"].includes(action) && (
            <Input
              label={
                action === "create" || action === "clone"
                  ? "New destination directory"
                  : "Repository directory"
              }
              value={path}
              onChange={(event) => setPath(event.target.value)}
              required
              autoComplete="off"
              hint={
                action === "create" || action === "clone"
                  ? "Absolute path. Parent must exist; destination must not exist."
                  : "Enter the repository root, including spaces or Unicode if present."
              }
            />
          )}
          {action && ["register", "create", "clone", "rename"].includes(action) && (
            <Input
              label="Display name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              required={action !== "register"}
              maxLength={240}
            />
          )}
          {action === "clone" && (
            <>
              <Input
                label="Git remote"
                required
                value={remote}
                onChange={(event) => setRemote(event.target.value)}
                hint="Authorized absolute local path, HTTPS or SSH URL. No embedded credentials."
              />
              <Checkbox
                label="Initialize Factory if this clone is not initialized"
                checked={initializeClone}
                onChange={(event) => setInitializeClone(event.target.checked)}
              />
              <Checkbox
                label="Authorize network clone using my existing Git authentication"
                checked={network}
                onChange={(event) => setNetwork(event.target.checked)}
              />
            </>
          )}
          {action === "relink" && (
            <Checkbox
              label="Accept a different filesystem or database identity for this project"
              checked={identityChange}
              onChange={(event) => setIdentityChange(event.target.checked)}
            />
          )}
          {needsTrust && (
            <Checkbox
              label="I trust this repository and its configured local tools"
              checked={trusted}
              onChange={(event) => setTrusted(event.target.checked)}
            />
          )}
          {error && <p role="alert">{error}</p>}
          <div className="projects-actions">
            <Button disabled={busy} onClick={() => setOperation(undefined)}>
              Cancel
            </Button>
            <Button
              type="submit"
              tone={action === "remove" ? "danger" : "primary"}
              loading={busy}
              disabled={needsTrust && !trusted}
            >
              {action ? titles[action] : "Apply"}
            </Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}
