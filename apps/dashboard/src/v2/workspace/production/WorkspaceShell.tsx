import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { Button, IconButton, Input, Select } from "../../design-system/primitives/Controls";
import {
  Dialog,
  ConfirmationDialog,
  Dropdown,
  Menu,
  type MenuEntry,
} from "../../design-system/primitives/Overlays";
import {
  ActivityBarItem,
  EmptyState,
  InlineAlert,
  LoadingIndicator,
  WorkspaceTab,
} from "../../design-system/components/Application";
import { Icon, type IconName } from "../../design-system/icons/Icon";
import { useTheme } from "../../design-system/themes/theme";
import {
  projectMutation,
  projectPath,
  type Project,
  type ApplicationInfo,
} from "../../projects/projectApi";
import { presets, titles, viewKinds, futureKinds, type ViewKind } from "./model";
import { WorkspaceModel, DockSession } from "./controller";
import { useData, refreshData } from "./data";
import { DockSurface } from "./DockSurface";
import { legacyUrl } from "./model";
import { browserPresence } from "../../lifecycle/presence";
import { WorkspaceLifecycleControls } from "../../lifecycle/LifecycleControls";
import { request } from "../../../api";
import { usePreferences, savePreferences, preferencesPath } from "../../product/preferences";
import {
  eligibleShortcut,
  shortcutConflicts,
  workspaceHasKeyboardFocus,
  type WorkspaceCommand,
} from "../../product/commands";
import type { ProductNotification } from "../../product/Notifications";
const CommandPalette = lazy(() =>
  import("../../product/CommandPalette").then((m) => ({ default: m.CommandPalette }))
);
const SettingsPanel = lazy(() =>
  import("../../product/SettingsPanel").then((m) => ({ default: m.SettingsPanel }))
);
const NotificationSignals = lazy(() =>
  import("../../product/Notifications").then((m) => ({ default: m.NotificationSignals }))
);
const activities: Array<{ kind: ViewKind; icon: IconName }> = [
  { kind: "explorer", icon: "files" },
  { kind: "search", icon: "search" },
  { kind: "sourceControl", icon: "git" },
  { kind: "agents", icon: "agents" },
  { kind: "memory", icon: "memory" },
  { kind: "workflows", icon: "play" },
  { kind: "debug", icon: "step" },
  { kind: "developmentTerminal", icon: "terminal" },
  { kind: "extensions", icon: "extensions" },
  { kind: "settings", icon: "settings" },
];
export function WorkspaceShell({ model }: { model: WorkspaceModel }) {
  const state = useSyncExternalStore(model.subscribe, model.snapshot),
    session = model.session,
    theme = useTheme();
  const preferences = usePreferences();
  const [palette, setPalette] = useState(false),
    [globalSettings, setGlobalSettings] = useState(false);
  const [preferenceBusy, setPreferenceBusy] = useState(false);
  const creatingConversation = useRef(false);
  const [unread, setUnread] = useState<Record<string, number | undefined>>({});
  const [toast, setToast] = useState<{ projectId: string; value: ProductNotification }>();
  const [pendingNavigation, setPendingNavigation] = useState<{
    projectId: string;
    kind: "workflow" | "review" | "agents" | "settings" | "sourceControl";
    resourceId?: number;
    ticket: number;
  }>();
  const handledNavigation = useRef<number>();
  const navigationResult = useRef<{
    ticket: number;
    resolve: () => void;
    reject: (error: Error) => void;
  }>();
  useEffect(
    () => () => navigationResult.current?.reject(new Error("Workspace navigation closed.")),
    []
  );
  const projects = useData<Project[]>("", "/projects"),
    application = useData<ApplicationInfo>("", "/application");
  const activeSessionId = session?.id;
  const presence = useSyncExternalStore(browserPresence.subscribe, browserPresence.get);
  useEffect(() => {
    browserPresence.start();
    return () => browserPresence.stop();
  }, []);
  useEffect(() => {
    if (state.ready)
      browserPresence.update(
        state.document.sessions
          .filter((s) =>
            projects.value?.some((p) => p.project_id === s.projectId && p.lifecycle === "open")
          )
          .map((s) => ({ projectId: s.projectId, sessionId: s.id }))
      );
  }, [state.ready, state.document.sessions, projects.value]);
  const dock = useMemo(
    () => (activeSessionId ? new DockSession(model, activeSessionId) : undefined),
    [model, activeSessionId]
  );
  const preferenceMigration = useRef(false);
  useEffect(() => {
    model.navigateFile = async (file, line, column) => {
      await dock?.openFile(file, "within", line ? { line, column } : undefined);
    };
    return () => {
      model.navigateFile = undefined;
    };
  }, [model, dock]);
  const [opening, setOpening] = useState(""),
    [authentication, setAuthentication] = useState("ready"),
    [replaceLocal, setReplaceLocal] = useState<"reload" | "empty" | null>(null),
    [resize, setResize] = useState(false),
    [width, setWidth] = useState(600),
    [height, setHeight] = useState(400),
    [rename, setRename] = useState(false),
    [sessionTitle, setSessionTitle] = useState(""),
    [menu, setMenu] = useState<{ x: number; y: number; id: string } | null>(null),
    [pin, setPin] = useState(false),
    [pinnedProject, setPinnedProject] = useState(""),
    [pinnedKind, setPinnedKind] = useState<ViewKind>("workflows");
  const switcher = useRef<HTMLDivElement>(null),
    app = useRef<HTMLDivElement>(null),
    loaded = useRef(false),
    openingRequest = useRef(0);
  useEffect(() => {
    const status = (event: Event) => setAuthentication((event as CustomEvent<string>).detail);
    window.addEventListener("factory-auth-status", status);
    return () => window.removeEventListener("factory-auth-status", status);
  }, []);
  useEffect(() => {
    if (!loaded.current) {
      loaded.current = true;
      void model.load();
    }
  }, [model]);
  useEffect(() => {
    const before = (event: BeforeUnloadEvent) => {
      if (preferenceBusy || model.state.pending || model.documents.needsUnloadWarning()) {
        event.preventDefault();
        event.returnValue = "Workspace changes have not reached durable storage.";
      }
    };
    const hide = () => {
      void model.save();
      for (const document of model.documents.all())
        if (document.dirty) void model.documents.persist(document.id);
    };
    window.addEventListener("beforeunload", before);
    window.addEventListener("pagehide", hide);
    return () => {
      window.removeEventListener("beforeunload", before);
      window.removeEventListener("pagehide", hide);
    };
  }, [model, preferenceBusy]);
  useEffect(() => {
    if (
      state.ready &&
      !state.document.activeProjectId &&
      !state.document.sessions.length &&
      !state.conflict &&
      (preferences.value || preferences.error) &&
      application.value?.default_project_id
    ) {
      model.defaultPreset = preferences.value?.preferences.default_layout ?? "Focus";
      model.activateProject(application.value.default_project_id);
    }
  }, [
    state.ready,
    state.conflict,
    state.document.activeProjectId,
    state.document.sessions.length,
    application.value?.default_project_id,
    preferences.value,
    preferences.error,
    model,
  ]);
  useEffect(() => {
    const envelope = preferences.value;
    if (!envelope || !state.ready) return;
    if (envelope.revision === 0) {
      if (state.conflict) return;
      if (!preferenceMigration.current) {
        preferenceMigration.current = true;
        void savePreferences({
          ...envelope,
          preferences: {
            ...envelope.preferences,
            theme: state.revision > 0 ? state.document.preferences.theme : theme.preference,
          },
        }).catch(() => void refreshData("", preferencesPath));
      }
      return;
    }
    if (theme.preference !== envelope.preferences.theme)
      theme.setPreference(envelope.preferences.theme);
    document.documentElement.dataset.density = envelope.preferences.density;
    model.defaultPreset = envelope.preferences.default_layout;
    if (!state.conflict && state.document.preferences.theme !== envelope.preferences.theme)
      model.updateDocument((doc) => ({
        ...doc,
        preferences: { theme: envelope.preferences.theme },
      }));
  }, [
    preferences.value,
    state.ready,
    state.revision,
    state.conflict,
    state.document.preferences.theme,
    theme,
    model,
  ]);
  const openProject = useCallback(
    async (id: string) => {
      if (!id || !model.state.ready) return;
      const ticket = ++openingRequest.current;
      setOpening(id);
      try {
        const project = projects.value?.find((p) => p.project_id === id);
        if (project?.lifecycle !== "open") await projectMutation(projectPath(id, "open"), "POST");
        if (ticket !== openingRequest.current) return;
        model.activateProject(id);
        model.announce("Project selected. Other project runtimes continue.");
        void refreshData("", "/projects");
      } catch (error) {
        if (ticket === openingRequest.current) model.setIssue(String(error));
      } finally {
        if (ticket === openingRequest.current) setOpening("");
      }
    },
    [model, projects.value]
  );
  const activeView = session?.views.find((v) => v.id === session.activeViewId);
  const commandProject = activeView?.projectId ?? session?.projectId;
  const projectRequired = !dock ? "Open a project workspace first." : undefined;
  const openKind = (kind: ViewKind) => {
    dock?.open(kind, commandProject ? { projectId: commandProject } : {});
    dock?.focus();
  };
  const openSettings = () => {
    if (dock) openKind("settings");
    else setGlobalSettings(true);
  };
  const commands: WorkspaceCommand[] = [
    {
      id: "commands",
      title: "Open command list",
      category: "Application",
      description: "Search available commands and keyboard shortcuts",
      shortcut: "Mod+Shift+p",
      terminal: true,
      run: () => setPalette(true),
    },
    {
      id: "project.open",
      title: "Open project",
      category: "Application",
      description: "Select a registered repository",
      shortcut: "Alt+Shift+p",
      run: () => switcher.current?.querySelector("select")?.focus(),
    },
    {
      id: "project.manage",
      title: "Register or create project",
      category: "Application",
      description: "Open the existing project manager",
      run: () => {
        window.location.assign("/projects.html");
      },
    },
    {
      id: "settings",
      title: "Open Settings",
      category: "Application",
      description: "Search application, project and workspace settings",
      shortcut: "Mod+,",
      run: openSettings,
    },
    {
      id: "notifications",
      title: "Open notifications",
      category: "Project",
      description: "Inspect recorded events and unread history",
      shortcut: "Alt+Shift+n",
      disabled: projectRequired,
      run: () => openKind("notifications"),
    },
    {
      id: "conversation",
      title: "Focus orchestrator conversation",
      category: "Project",
      description: "Open the current conversation without submitting an objective",
      shortcut: "Alt+Shift+o",
      disabled: projectRequired,
      run: () => openKind("home"),
    },
    {
      id: "mission.new",
      title: "New mission",
      category: "Project",
      description: "Open the mission form; execution requires explicit submission",
      disabled: projectRequired,
      run: async () => {
        if (!dock || !commandProject || creatingConversation.current) return;
        creatingConversation.current = true;
        const ticket = ++openingRequest.current;
        try {
          const conversation = await request<{ id: string }>(
            projectPath(commandProject, "conversations"),
            {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ title: "Development conversation" }),
            }
          );
          void refreshData(commandProject, "conversations?limit=100");
          if (ticket !== openingRequest.current || model.session?.id !== dock.sessionId) return;
          const id = dock.open("home", { projectId: commandProject });
          if (id)
            model.updateView(dock.sessionId, id, {
              missionMode: "form",
              conversationId: conversation.id,
            });
          dock.focus();
        } finally {
          creatingConversation.current = false;
        }
      },
    },
    {
      id: "file.open",
      title: "Quick open file",
      category: "Project",
      description: "Search authorized file paths",
      shortcut: "Mod+p",
      disabled: projectRequired,
      run: () => {
        const id = dock?.open("search", { projectId: commandProject });
        if (id && dock) model.updateView(dock.sessionId, id, { searchContent: false });
        dock?.focus();
      },
    },
    {
      id: "search.files",
      title: "Search files",
      category: "Project",
      description: "Search authorized names and paths",
      disabled: projectRequired,
      run: () => openKind("search"),
    },
    {
      id: "search.content",
      title: "Search project content",
      category: "Project",
      description: "Search content through the bounded filesystem service",
      shortcut: "Mod+Shift+f",
      disabled: projectRequired,
      run: () => {
        const id = dock?.open("search", { projectId: commandProject });
        if (id && dock) model.updateView(dock.sessionId, id, { searchContent: true });
        dock?.focus();
      },
    },
    ...(
      [
        ["sourceControl", "Open Source Control"],
        ["developmentTerminal", "Focus development terminal"],
        ["agents", "Open Agents"],
        ["memory", "Open Memory"],
        ["diagnostics", "Open diagnostics"],
        ["extensions", "Open Extensions"],
      ] as const
    ).map(([kind, title]) => ({
      id: `view.${kind}`,
      title,
      category: "Project" as const,
      description: `Open ${titles[kind]} for the selected view's project`,
      disabled: projectRequired,
      shortcut: kind === "developmentTerminal" ? "Mod+backquote" : undefined,
      run: () => openKind(kind),
    })),
    {
      id: "file.save",
      title: "Save current file",
      category: "Editor",
      description: "Save through the revision-aware document service",
      shortcut: "Mod+s",
      disabled:
        !activeView?.documentId || !activeView.file ? "Focus a repository file editor." : undefined,
      run: () => {
        if (activeView?.documentId && activeView.file)
          void model.documents.save(activeView.documentId, activeView.file);
      },
    },
    {
      id: "file.saveAll",
      title: "Save all files",
      category: "Editor",
      description: "Save dirty documents with independent per-file results",
      shortcut: "Mod+Shift+s",
      disabled: projectRequired,
      run: () => {
        void model.documents.saveAll();
      },
    },
    {
      id: "file.find",
      title: "Find in current file",
      category: "Editor",
      description: "Use CodeMirror search in the focused source pane",
      disabled: !activeView?.file ? "Focus a repository file editor." : undefined,
      run: () => {
        if (activeView)
          window.dispatchEvent(
            new CustomEvent("factory-editor-find", {
              detail: { viewId: activeView.id, projectId: activeView.projectId },
            })
          );
      },
    },
    {
      id: "group.next",
      title: "Focus next panel group",
      category: "Workspace",
      description: "Keyboard alternative to docking",
      shortcut: "F6",
      terminal: true,
      disabled: projectRequired,
      run: () => dock?.focusNext(1),
    },
    {
      id: "group.previous",
      title: "Focus previous panel group",
      category: "Workspace",
      description: "Keyboard alternative to docking",
      shortcut: "Shift+F6",
      terminal: true,
      disabled: projectRequired,
      run: () => dock?.focusNext(-1),
    },
    {
      id: "tab.next",
      title: "Next workspace tab",
      category: "Workspace",
      description: "Keeps browser tab shortcuts available",
      shortcut: "Alt+PageDown",
      disabled: projectRequired,
      run: () => dock?.nextTab(1),
    },
    {
      id: "tab.previous",
      title: "Previous workspace tab",
      category: "Workspace",
      description: "Keeps browser tab shortcuts available",
      shortcut: "Alt+PageUp",
      disabled: projectRequired,
      run: () => dock?.nextTab(-1),
    },
    {
      id: "tab.close",
      title: "Close current presentation tab",
      category: "Workspace",
      description: "Retains recoverable drafts and does not suspend missions",
      shortcut: "Alt+Shift+w",
      disabled: projectRequired,
      run: () => dock?.requestClose(),
    },
    {
      id: "sidebar.toggle",
      title: "Toggle sidebar",
      category: "Workspace",
      description: "Hide or restore the Explorer view",
      shortcut: "Alt+Shift+b",
      disabled: projectRequired,
      run: () => {
        const v = dock?.session.views.find((v) => v.kind === "explorer" && !v.closed);
        if (v && !v.hidden) dock?.close(v.id, true);
        else if (v) dock?.restore(v.id);
        else openKind("explorer");
      },
    },
    {
      id: "bottom.toggle",
      title: "Toggle bottom output panel",
      category: "Workspace",
      description: "Hide or restore recorded agent output",
      disabled: projectRequired,
      run: () => {
        const v = dock?.session.views.find((v) => v.kind === "console" && !v.closed);
        if (v && !v.hidden) dock?.close(v.id, true);
        else if (v) dock?.restore(v.id);
        else dock?.open("console", {}, "below");
      },
    },
    {
      id: "view.context",
      title: "Open active view actions",
      category: "Workspace",
      description: "Move, resize or close the current view",
      shortcut: "Shift+F10",
      disabled: projectRequired,
      run: () => {
        const panel = dock?.api?.activePanel;
        if (panel) {
          const box = panel.view.tab.element.getBoundingClientRect();
          setMenu({ id: panel.id, x: box.x, y: box.bottom });
        }
      },
    },
    ...presets.map((preset, i) => ({
      id: `layout.${preset}`,
      title: `Switch to ${preset} layout`,
      category: "Workspace" as const,
      description: "Preserve open resources and drafts",
      shortcut: `Alt+Shift+${i + 1}`,
      disabled: projectRequired,
      run: () => dock?.preset(preset),
    })),
    ...(projects.value ?? []).map((p) => ({
      id: `project.${p.project_id}`,
      title: `Switch project: ${p.display_name}`,
      category: "Application" as const,
      description: p.project_id,
      disabled:
        p.availability !== "available" ? (p.problem ?? "Project is unavailable.") : undefined,
      run: () => openProject(p.project_id),
    })),
  ];
  const commandRef = useRef(commands);
  commandRef.current = commands;
  useEffect(() => {
    const keyboard = (event: KeyboardEvent) => {
      if (!workspaceHasKeyboardFocus(app.current)) return;
      const terminal = !!(event.target instanceof Element && event.target.closest(".xterm"));
      const command = eligibleShortcut(event, commandRef.current, terminal);
      if (command) {
        event.preventDefault();
        void Promise.resolve(command.run()).catch((e) => model.setIssue(String(e)));
      }
    };
    window.addEventListener("keydown", keyboard);
    return () => window.removeEventListener("keydown", keyboard);
  }, [model]);
  useEffect(() => {
    model.navigateResource = async (target) => {
      const project = projects.value?.find((p) => p.project_id === target.projectId);
      if (!project || project.availability !== "available" || project.lifecycle !== "open")
        throw new Error(
          "The target project is missing or closed. Open its registered runtime before navigating."
        );
      if (
        target.resourceId !== undefined &&
        (!Number.isSafeInteger(target.resourceId) || target.resourceId <= 0)
      )
        throw new Error("Invalid resource identity.");
      const ticket = ++openingRequest.current;
      const sourceSession = model.session?.id;
      navigationResult.current?.reject(
        new Error("A newer navigation request replaced this target.")
      );
      navigationResult.current = undefined;
      if (target.resourceId)
        await request(projectPath(target.projectId, `runs/${target.resourceId}`));
      if (ticket !== openingRequest.current || model.session?.id !== sourceSession)
        throw new Error(
          "Navigation changed while the target was loading. Open the notification again."
        );
      model.activateProject(target.projectId);
      await new Promise<void>((resolve, reject) => {
        navigationResult.current = { ticket, resolve, reject };
        setPendingNavigation({ ...target, ticket });
      });
    };
    return () => {
      model.navigateResource = undefined;
    };
  }, [model, projects.value]);
  useEffect(() => {
    if (!pendingNavigation) return;
    if (pendingNavigation.ticket !== openingRequest.current) {
      navigationResult.current?.reject(
        new Error("Navigation changed before this target could open.")
      );
      navigationResult.current = undefined;
      setPendingNavigation(undefined);
      return;
    }
    if (dock?.api && dock.session.projectId === pendingNavigation.projectId) {
      if (handledNavigation.current === pendingNavigation.ticket) return;
      handledNavigation.current = pendingNavigation.ticket;
      const result = navigationResult.current;
      navigationResult.current = undefined;
      setPendingNavigation(undefined);
      try {
        const id = dock.open(pendingNavigation.kind, {
          projectId: pendingNavigation.projectId,
          resourceId: pendingNavigation.resourceId,
        });
        if (!id) throw new Error("The target view could not be opened.");
        dock.focus();
        result?.resolve();
      } catch (error) {
        result?.reject(error instanceof Error ? error : new Error(String(error)));
      }
    }
  }, [pendingNavigation, dock, state.document]);
  const onUnread = useCallback(
    (project: string, count: number | undefined) =>
      setUnread((old) => (old[project] === count ? old : { ...old, [project]: count })),
    []
  );
  const onArrival = useCallback(
    (projectId: string, value: ProductNotification) => setToast({ projectId, value }),
    []
  );
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(undefined), 8000);
    return () => clearTimeout(timer);
  }, [toast]);
  const focusMenu = useCallback(
    (event: MouseEvent, id: string) => {
      dock?.api?.getPanel(id)?.api.setActive();
      setMenu({ x: event.clientX, y: event.clientY, id });
    },
    [dock]
  );
  const viewCommands: MenuEntry[] = dock
    ? [
        { id: "left", label: "Move tab left", action: () => dock.move("left") },
        { id: "right", label: "Move tab right", action: () => dock.move("right") },
        {
          id: "group",
          label: "Move to next group",
          disabled: (dock.api?.groups.length ?? 0) < 2,
          action: () => dock.moveGroup(),
        },
        { id: "split-right", label: "Split vertically", action: () => dock.split("right") },
        { id: "split-below", label: "Split horizontally", action: () => dock.split("below") },
        {
          id: "resize",
          label: "Resize active panel",
          action: () => {
            setWidth(dock.api?.activePanel?.api.width ?? 600);
            setHeight(dock.api?.activePanel?.api.height ?? 400);
            setResize(true);
          },
        },
        {
          id: "maximize",
          label: "Toggle maximization",
          action: () => {
            const panel = dock.api?.activePanel;
            if (panel?.api.isMaximized()) panel.api.exitMaximized();
            else panel?.api.maximize();
          },
        },
        {
          id: "hide",
          label: "Hide active view",
          action: () => {
            const id = dock.api?.activePanel?.id;
            if (id) dock.close(id, true);
          },
        },
        {
          id: "close",
          label: "Close active view",
          shortcut: "Alt+Shift+W",
          action: () => dock.requestClose(),
        },
      ]
    : [];
  const sessionCommands: MenuEntry[] = session
    ? [
        {
          id: "rename-session",
          label: "Rename workspace session",
          action: () => {
            setSessionTitle(session.title);
            setRename(true);
          },
        },
        {
          id: "hide-session",
          label: "Hide session from navigation",
          action: () => model.archiveSession(),
        },
      ]
    : [];
  const menuEntries: MenuEntry[] = viewKinds
    .filter((kind) => !["workflow", "terminal"].includes(kind))
    .map((kind) => ({
      id: kind,
      label: titles[kind],
      disabled: futureKinds.includes(kind) || !dock,
      action: () => dock?.open(kind, {}, "within", kind === "scratch"),
    }));
  const sessionOptions = state.document.sessions.filter(
    (s) => s.projectId === state.document.activeProjectId
  );
  const selectedProject = projects.value?.find(
    (p) => p.project_id === state.document.activeProjectId
  );
  const notificationProjects = state.document.openProjects.filter((id) =>
    projects.value?.some((p) => p.project_id === id && p.lifecycle === "open")
  );
  const unreadLabel = notificationProjects.every((id) => unread[id] !== undefined)
    ? String(notificationProjects.reduce((n, id) => n + (unread[id] ?? 0), 0))
    : "loading or unavailable";
  return (
    <div className="asf-v2 ws-app" ref={app}>
      <header className="ws-top">
        <a className="ws-brand" href="/workspace.html" aria-label="ASF workspace">
          <Icon name="code" />
          ASF
        </a>
        <div ref={switcher} className="ws-project-switch">
          <Select
            label="Open registered project"
            hideLabel
            value=""
            disabled={!state.ready}
            onChange={(event) => void openProject(event.target.value)}
          >
            <option value="">Open project...</option>
            {projects.value?.map((project) => (
              <option key={project.project_id} value={project.project_id}>
                {project.display_name}
                {project.availability !== "available" ? " · unavailable" : ""}
              </option>
            ))}
          </Select>
        </div>
        <div
          className="ws-project-tabs"
          role="tablist"
          aria-label="Open projects"
          onKeyDown={(event) => {
            if (event.altKey || event.ctrlKey || event.metaKey) return;
            if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
              event.preventDefault();
              const tabs = state.document.openProjects,
                index = tabs.indexOf(state.document.activeProjectId ?? ""),
                next =
                  tabs[(index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length];
              if (next) {
                openingRequest.current++;
                model.activateProject(next);
                requestAnimationFrame(() => document.getElementById(`project-${next}`)?.focus());
              }
            }
          }}
        >
          {state.document.openProjects.map((id) => (
            <div
              key={id}
              draggable
              onDragStart={(event) => event.dataTransfer.setData("application/x-asf-project", id)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => {
                event.preventDefault();
                const source = event.dataTransfer.getData("application/x-asf-project");
                if (source !== id && state.document.openProjects.includes(source))
                  model.reorderProjects(
                    source,
                    state.document.openProjects.indexOf(id) -
                      state.document.openProjects.indexOf(source)
                  );
              }}
            >
              <WorkspaceTab
                id={`project-${id}`}
                label={
                  projects.value?.find((p) => p.project_id === id)?.display_name ??
                  "Missing project"
                }
                selected={id === state.document.activeProjectId}
                onSelect={() => {
                  openingRequest.current++;
                  setOpening("");
                  model.activateProject(id);
                }}
                onClose={() => model.hideProject(id)}
                closeLabel="hide this project tab; backend work continues"
                onMove={(direction) => model.reorderProjects(id, direction === "left" ? -1 : 1)}
              />
            </div>
          ))}
        </div>
        <span className="ws-connection" role="status">
          {authentication === "refreshing"
            ? "Refreshing browser session"
            : authentication === "failed"
              ? "Authentication unavailable"
              : projects.error
                ? "API unavailable"
                : opening
                  ? "Opening project"
                  : projects.value
                    ? "Local connection"
                    : "Connecting"}
        </span>
        <Button tone="quiet" className="ws-command-access" onClick={() => setPalette(true)}>
          Commands
        </Button>
        <IconButton
          label={`Notifications (${unreadLabel} unread)`}
          icon="bell"
          disabled={!dock}
          onClick={() => openKind("notifications")}
        />
        <IconButton label="Open Settings" icon="settings" onClick={openSettings} />
        <a href="/projects.html" className="ws-link">
          Manage projects
        </a>
        <a
          href={selectedProject ? legacyUrl(selectedProject.project_id) : "/"}
          className="ws-link"
          target="_blank"
          rel="noopener"
        >
          V1
        </a>
      </header>
      <div className="ws-session-bar">
        <div
          className="ws-session-tabs"
          role="tablist"
          aria-label="Workspace sessions; navigation only"
          onKeyDown={(event) => {
            if (event.altKey || event.ctrlKey || event.metaKey) return;
            const peers = sessionOptions.filter((item) => !item.archived),
              index = peers.findIndex((item) => item.id === state.document.activeSessionId);
            const next =
              event.key === "ArrowRight"
                ? peers[(index + 1) % peers.length]
                : event.key === "ArrowLeft"
                  ? peers[(index - 1 + peers.length) % peers.length]
                  : event.key === "Home"
                    ? peers[0]
                    : event.key === "End"
                      ? peers.at(-1)
                      : undefined;
            if (next) {
              event.preventDefault();
              model.activateSession(next.id);
              requestAnimationFrame(() => document.getElementById(`session-${next.id}`)?.focus());
            }
          }}
        >
          {sessionOptions
            .filter((s) => !s.archived)
            .map((s) => (
              <div
                key={s.id}
                draggable
                onDragStart={(event) =>
                  event.dataTransfer.setData("application/x-asf-session", s.id)
                }
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  event.preventDefault();
                  const source = event.dataTransfer.getData("application/x-asf-session");
                  if (sessionOptions.some((item) => item.id === source))
                    model.reorderSessions(
                      source,
                      sessionOptions.findIndex((item) => item.id === s.id) -
                        sessionOptions.findIndex((item) => item.id === source)
                    );
                }}
              >
                <WorkspaceTab
                  id={`session-${s.id}`}
                  label={s.title}
                  selected={s.id === state.document.activeSessionId}
                  onSelect={() => model.activateSession(s.id)}
                  onClose={() => model.archiveSession(s.id)}
                  closeLabel="hide this navigation session; backend work continues"
                  onMove={(direction) => model.reorderSessions(s.id, direction === "left" ? -1 : 1)}
                />
              </div>
            ))}
        </div>
        <Select
          label="Workspace session"
          hideLabel
          value={state.document.activeSessionId ?? ""}
          onChange={(event) => {
            openingRequest.current++;
            model.activateSession(event.target.value);
          }}
        >
          <option value="">Workspace session</option>
          {sessionOptions.map((s) => (
            <option key={s.id} value={s.id}>
              {s.title}
              {s.archived ? " · hidden" : ""}
            </option>
          ))}
        </Select>
        <IconButton
          label="New workspace session"
          icon="plus"
          disabled={!state.document.activeProjectId}
          onClick={() => model.newSession()}
        />
        <Button
          tone="quiet"
          className="ws-session-secondary"
          disabled={!session}
          onClick={() => {
            setSessionTitle(session?.title ?? "");
            setRename(true);
          }}
        >
          Rename
        </Button>
        <Button
          className="ws-session-secondary"
          tone="quiet"
          disabled={!session}
          onClick={() => model.archiveSession()}
        >
          Hide session
        </Button>
        {session && (
          <WorkspaceLifecycleControls
            key={`${session.projectId}:${session.id}`}
            projectId={session.projectId}
            sessionId={session.id}
          />
        )}
        <div className="ws-presets" role="group" aria-label="Layout presets">
          {presets.map((preset) => (
            <Button
              key={preset}
              tone="quiet"
              aria-pressed={session?.preset === preset}
              disabled={!dock}
              onClick={() => dock?.preset(preset)}
            >
              {preset}
            </Button>
          ))}
        </div>
        <Dropdown
          label="Views"
          entries={[
            ...menuEntries,
            ...(session?.views
              .filter((v) => v.closed || v.hidden)
              .map((v) => ({
                id: `reopen-${v.id}`,
                label: `Reopen ${v.title}`,
                action: () => dock?.restore(v.id),
              })) ?? []),
            {
              id: "pin",
              label: "Open another project's view beside this",
              disabled: !dock,
              action: () => setPin(true),
            },
          ]}
        />
        <Dropdown
          label="Layout actions"
          entries={
            viewCommands.length
              ? [...viewCommands, ...sessionCommands]
              : [{ id: "none", label: "Open a project first", disabled: true, action: () => {} }]
          }
        />
        <Select
          label="Theme"
          hideLabel
          value={theme.preference}
          disabled={!preferences.value || preferenceBusy}
          onChange={(event) => {
            if (!preferences.value) return;
            setPreferenceBusy(true);
            void savePreferences({
              ...preferences.value,
              preferences: {
                ...preferences.value.preferences,
                theme: event.target.value as "system" | "light" | "dark",
              },
            })
              .catch((e) => model.setIssue(String(e)))
              .finally(() => setPreferenceBusy(false));
          }}
        >
          <option value="system">System</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </Select>
      </div>
      {(state.issue || theme.issue || projects.error || preferences.error) && (
        <div className="ws-recovery">
          <InlineAlert
            tone="warning"
            title="Workspace needs attention"
            onDismiss={() => model.setIssue("")}
          >
            <span>{state.issue || theme.issue || projects.error || preferences.error}</span>
            <div className="ws-actions">
              <Button
                onClick={() => {
                  void refreshData("", "/projects");
                  void refreshData("", "/application");
                  model.retrySave();
                }}
              >
                Reconnect / retry save
              </Button>
              {state.conflict && (
                <>
                  <Button onClick={() => model.independentCopy()}>Save independent copy</Button>
                  <Button
                    disabled={state.saving}
                    onClick={() =>
                      state.pending ? setReplaceLocal("reload") : void model.reloadSaved()
                    }
                  >
                    Reload saved workspace
                  </Button>
                  <Button
                    disabled={state.saving}
                    onClick={() =>
                      state.pending ? setReplaceLocal("empty") : model.recoverEmpty()
                    }
                  >
                    Open safe empty workspace
                  </Button>
                </>
              )}
            </div>
          </InlineAlert>
        </div>
      )}
      <div className="ws-main">
        <nav className="ws-rail" aria-label="Workspace activities">
          {activities.map((activity) => (
            <ActivityBarItem
              key={activity.kind}
              icon={activity.icon}
              label={titles[activity.kind]}
              selected={session?.activity === activity.kind}
              disabled={!dock}
              onClick={() => {
                if (dock) {
                  model.updateSession(dock.sessionId, (s) => ({ ...s, activity: activity.kind }));
                  dock.open(activity.kind);
                }
              }}
            />
          ))}
          <ActivityBarItem
            icon="terminal"
            label="Existing agent sessions"
            disabled={!dock}
            onClick={() => dock?.open("console", {}, "below")}
          />
        </nav>
        <main className="ws-workarea">
          {!state.ready ? (
            <LoadingIndicator label="Restoring workspace" />
          ) : dock ? (
            <DockSurface key={dock.sessionId} model={model} dock={dock} contextMenu={focusMenu} />
          ) : (
            <div className="ws-empty">
              <EmptyState
                title="Open a project"
                description="Choose an actual registered repository to create a workspace session. Layouts and navigation persist independently of running workflows."
                action={<a href="/projects.html">Register, create or clone a project</a>}
              />
              {projects.error && <p role="alert">{projects.error}</p>}
            </div>
          )}
        </main>
      </div>
      <footer className="ws-status">
        <span title={state.document.id}>
          {selectedProject?.display_name ?? "No active project"}
        </span>
        <span>{session?.title ?? "No session"}</span>
        <span>{activeView?.file?.path ?? activeView?.title ?? "No active view"}</span>
        <span title={presence.error}>
          {presence.connected ? "Browser lease active" : "Browser presence reconnecting"}
        </span>
        <span className="ws-status-spacer" />
        <span role="status">
          {!preferences.value && !preferences.error
            ? "Loading preferences..."
            : preferenceBusy
              ? "Saving preferences..."
              : state.saving
                ? "Saving layout..."
                : state.pending
                  ? "Layout not saved"
                  : state.revision
                    ? "Workspace saved"
                    : "New workspace"}
        </span>
        <Button tone="quiet" onClick={() => model.independentCopy()}>
          Independent workspace
        </Button>
        <span className="ds-sr-only" aria-live="polite">
          {state.notice}
        </span>
      </footer>
      <Suspense fallback={null}>
        {palette ? (
          <CommandPalette
            open
            onClose={() => setPalette(false)}
            onError={(error) => model.setIssue(String(error))}
            commands={commands}
          />
        ) : null}
        {state.document.openProjects
          .filter((id) =>
            projects.value?.some((p) => p.project_id === id && p.lifecycle === "open")
          )
          .map((id) => (
            <NotificationSignals
              key={id}
              projectId={id}
              onUnread={onUnread}
              onArrival={onArrival}
            />
          ))}
        <Dialog
          open={globalSettings}
          onClose={() => setGlobalSettings(false)}
          title="Application settings"
        >
          <SettingsPanel model={model} />
        </Dialog>
      </Suspense>
      {toast ? (
        <aside className="product-toast" role="status">
          <strong>{toast.value.title}</strong>
          <div className="product-actions">
            <Button
              onClick={() => {
                void model
                  .navigateResource?.({
                    projectId: toast.projectId,
                    kind: "workflow",
                    resourceId: toast.value.run_id ?? undefined,
                  })
                  .catch((e) => model.setIssue(String(e)));
                setToast(undefined);
              }}
            >
              Open event
            </Button>
            <Button onClick={() => setToast(undefined)}>Dismiss brief notification</Button>
          </div>
        </aside>
      ) : null}
      {shortcutConflicts(commands).length ? (
        <span role="alert">
          Conflicting command shortcuts are disabled. Open the command list to use their actions.
        </span>
      ) : null}
      {menu && (
        <Menu
          label="View actions"
          entries={viewCommands}
          anchor={menu}
          onClose={() => {
            setMenu(null);
            dock?.focus();
          }}
        />
      )}
      <Dialog
        open={resize}
        onClose={() => setResize(false)}
        title="Resize active panel"
        description="Dimensions are bounded by the viewport and neighboring panel minima."
        footer={
          <Button
            onClick={() => {
              dock?.resize(width, height);
              setResize(false);
            }}
          >
            Apply size
          </Button>
        }
      >
        <Input
          label="Width in pixels"
          type="number"
          min={160}
          max={window.innerWidth}
          value={width}
          onChange={(event) => setWidth(Number(event.target.value))}
        />
        <Input
          label="Height in pixels"
          type="number"
          min={100}
          max={window.innerHeight}
          value={height}
          onChange={(event) => setHeight(Number(event.target.value))}
        />
        <div className="ws-actions">
          <Button onClick={() => setWidth((v) => v - 32)}>Narrower</Button>
          <Button onClick={() => setWidth((v) => v + 32)}>Wider</Button>
        </div>
      </Dialog>
      <Dialog
        open={rename}
        onClose={() => setRename(false)}
        title="Rename workspace session"
        footer={
          <Button
            disabled={!sessionTitle.trim()}
            onClick={() => {
              if (session)
                model.updateSession(session.id, (s) => ({ ...s, title: sessionTitle.trim() }));
              setRename(false);
            }}
          >
            Rename session
          </Button>
        }
      >
        <Input
          label="Session title"
          maxLength={120}
          value={sessionTitle}
          onChange={(event) => setSessionTitle(event.target.value)}
        />
      </Dialog>
      <Dialog
        open={pin}
        onClose={() => setPin(false)}
        title="Open project view beside current"
        description="This explicit cross-project view retains its own project identity. It does not inject that project's data into an agent context."
        footer={
          <Button
            disabled={!pinnedProject}
            onClick={() => {
              dock?.open(
                pinnedKind,
                {
                  projectId: pinnedProject,
                  title: `${titles[pinnedKind]} · ${projects.value?.find((p) => p.project_id === pinnedProject)?.display_name}`,
                },
                "right",
                true
              );
              setPin(false);
            }}
          >
            Open beside current
          </Button>
        }
      >
        <Select
          label="Project view type"
          value={pinnedKind}
          onChange={(event) => setPinnedKind(event.target.value as ViewKind)}
        >
          <option value="workflows">Workflows</option>
          <option value="agents">Agents and sessions</option>
          <option value="home">Orchestrator conversation</option>
        </Select>
        <Select
          label="Project for this view"
          value={pinnedProject}
          onChange={(event) => setPinnedProject(event.target.value)}
        >
          <option value="">Select open project</option>
          {projects.value
            ?.filter((p) => p.lifecycle === "open")
            .map((p) => (
              <option key={p.project_id} value={p.project_id}>
                {p.display_name}
              </option>
            ))}
        </Select>
      </Dialog>
      <Dialog
        open={
          !!state.confirmClose &&
          !!session?.views.find((view) => view.id === state.confirmClose)?.documentId
        }
        onClose={() => {
          model.state = { ...model.state, confirmClose: null };
          model.announce("Draft remains open.");
        }}
        title="Unsaved changes"
        footer={
          <>
            <Button
              onClick={() => {
                model.state = { ...model.state, confirmClose: null };
                model.announce("Close cancelled.");
              }}
            >
              Cancel
            </Button>
            <Button
              onClick={() => {
                const view = session?.views.find((view) => view.id === state.confirmClose);
                if (view?.documentId)
                  void model.documents
                    .discard(view.documentId)
                    .then(() => dock?.close(view.id))
                    .catch((error) => model.setIssue(String(error)));
                else if (view) dock?.close(view.id);
              }}
            >
              Discard changes and close
            </Button>
            <Button
              onClick={() => {
                const view = session?.views.find((view) => view.id === state.confirmClose);
                if (view?.documentId)
                  void model.documents.persist(view.documentId).then(() => {
                    const document = model.documents.get(view.documentId!);
                    if (document?.recoverySaved) dock?.close(view.id);
                    else
                      model.setIssue(
                        "Recovery has not been confirmed. Keep the view open and retry."
                      );
                  });
                else if (view) dock?.close(view.id);
              }}
            >
              Keep draft and close view
            </Button>
            <Button
              tone="primary"
              onClick={() => {
                const view = session?.views.find((view) => view.id === state.confirmClose);
                if (view?.documentId)
                  void model.documents.save(view.documentId).then((saved) => {
                    if (saved) dock?.close(view.id);
                  });
                else if (view) dock?.close(view.id);
              }}
            >
              Save and close
            </Button>
          </>
        }
      >
        <p>
          Closing this view does not stop a workflow. Save writes the repository only after backend
          confirmation. Keeping a draft preserves it separately from disk.
        </p>
      </Dialog>
      <ConfirmationDialog
        open={
          !!state.confirmClose &&
          !session?.views.find((view) => view.id === state.confirmClose)?.documentId
        }
        onClose={() => {
          model.state = { ...model.state, confirmClose: null };
          model.announce("Draft remains open.");
        }}
        onConfirm={() => {
          if (state.confirmClose) dock?.close(state.confirmClose);
        }}
        title="Close view with a retained draft?"
        description="The unsaved workspace note remains recoverable from Views. No repository file is written and no backend operation is stopped."
        confirmLabel="Keep draft and close view"
      />
      <ConfirmationDialog
        open={!!replaceLocal}
        onClose={() => setReplaceLocal(null)}
        onConfirm={() => {
          if (replaceLocal === "reload") void model.reloadSaved();
          else model.recoverEmpty();
          setReplaceLocal(null);
        }}
        title="Replace local presentation changes?"
        description="Unsaved layout changes and drafts in this window will be discarded. Cancel and save an independent copy to retain them. Server workspaces and running operations remain intact."
        confirmLabel="Replace local changes"
      />
    </div>
  );
}
