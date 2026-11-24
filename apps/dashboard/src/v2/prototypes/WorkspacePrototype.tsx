import { useEffect, useRef, useState } from "react";
import {
  ActivityBarItem,
  EmptyState,
  InlineAlert,
  NotificationItem,
  StatusBarItem,
  WorkspaceTab,
} from "../design-system/components/Application";
import {
  Button,
  IconButton,
  Input,
  SearchField,
  Select,
  Tabs,
  Toggle,
} from "../design-system/primitives/Controls";
import {
  ConfirmationDialog,
  Dialog,
  Dropdown,
  Tooltip,
  type MenuEntry,
} from "../design-system/primitives/Overlays";
import { DiagnosticsIndicator, LanguageLabel } from "../design-system/patterns/Code";
import { Icon } from "../design-system/icons/Icon";
import { useTheme } from "../design-system/themes/theme";
import { WorkspaceController } from "../workspace/controller";
import { DockSurface } from "../workspace/shell/DockSurface";
import { WorkspaceContext, useWorkspace, useWorkspaceState } from "../workspace/shell/context";
import { viewTitles, type Preset, type ViewName } from "../workspace/layouts/presets";
import { scenarios } from "./fixtures/data";
import { ComponentGallery } from "./ComponentGallery";
import "../design-system/components/system.css";
import "../workspace/shell/workspace.css";

function SearchDialog({ open, onClose }: { open: boolean; onClose(): void }) {
  const model = useWorkspace();
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const commands = (["Focus", "Code", "Agents", "Review"] as Preset[]).map((preset) => ({
    id: `preset-${preset}`,
    label: `${preset} layout`,
    detail: "Workspace preset",
    run: () => model.setPreset(preset),
    icon: "layout" as const,
  }));
  const entries = [
    ...commands,
    ...model.files.map((file) => ({
      id: file.path,
      label: file.path,
      detail: file.language,
      run: () => model.openFile(file.path),
      icon: "file" as const,
    })),
  ]
    .filter((item) => item.label.toLowerCase().includes(query.toLowerCase()))
    .slice(0, 30);
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelected(0);
      input.current?.focus();
    }
  }, [open]);
  const select = (index: number) => {
    entries[index]?.run();
    onClose();
  };
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Search workspace"
      description="Open a fixture file or switch a workspace layout."
    >
      <SearchField
        label="Search fixture files and commands"
        placeholder="Search files and layouts…"
        value={query}
        onChange={(value) => {
          setQuery(value);
          setSelected(0);
        }}
        inputRef={input}
        controls="command-results"
        activeDescendant={entries.length ? `command-${selected}` : undefined}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setSelected(
              (previous) =>
                (previous + (event.key === "ArrowDown" ? 1 : entries.length - 1)) %
                Math.max(entries.length, 1)
            );
          } else if (event.key === "Enter") {
            event.preventDefault();
            select(selected);
          }
        }}
      />
      <div
        id="command-results"
        role="listbox"
        aria-label="Fixture files and commands"
        className="v2-command-results"
      >
        {entries.map((entry, index) => (
          <button
            key={entry.id}
            id={`command-${index}`}
            role="option"
            aria-selected={index === selected}
            type="button"
            tabIndex={-1}
            title={entry.label}
            onMouseMove={() => setSelected(index)}
            onClick={() => select(index)}
          >
            <Icon name={entry.icon} />
            <span>{entry.label}</span>
            <small>{entry.detail}</small>
          </button>
        ))}
      </div>
      {!entries.length && (
        <EmptyState
          title="No matching file or command"
          description="Try a shorter name or a workspace preset."
        />
      )}
      <p className="v2-fine-print">
        ↑ ↓ to navigate · Enter to open · Escape to close · up to 30 matching entries
      </p>
    </Dialog>
  );
}

function WorkspaceChrome({ onGallery }: { onGallery(): void }) {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const theme = useTheme();
  const activePanel = model.api?.activePanel;
  const activeIndex = activePanel ? activePanel.group.panels.indexOf(activePanel) : -1;
  const [search, setSearch] = useState(false);
  const [settings, setSettings] = useState(false);
  const [notifications, setNotifications] = useState(false);
  const [reset, setReset] = useState(false);
  const [resize, setResize] = useState(false);
  const [dimensions, setDimensions] = useState({ width: "600", height: "400" });
  const [compact, setCompact] = useState(() => {
    try {
      return localStorage.getItem("asf.v2.p02.density") === "compact";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    document.documentElement.dataset.density = compact ? "compact" : "default";
  }, [compact]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setSearch(true);
      }
      if (event.key === "F6" && !document.querySelector("dialog[open]")) {
        event.preventDefault();
        model.focusNext();
      }
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [model]);
  const switchProject = (project: string) => {
    model.persist();
    const query = new URLSearchParams(location.search);
    query.set("project", project);
    location.search = query.toString();
  };
  const views: MenuEntry[] = (
    ["explorer", "conversation", "mission", "agents", "output", "review"] as ViewName[]
  ).map((view) => ({
    id: view,
    label: `${model.hasView(view) ? "Hide" : "Show"} ${viewTitles[view]}`,
    icon: "layout" as const,
    action: () => model.toggleView(view),
  }));
  views.push({
    id: "resize",
    label: "Resize active panel…",
    icon: "layout",
    action: () => {
      const panel = model.api?.activePanel;
      if (!panel) return;
      setDimensions({
        width: String(Math.round(panel.api.width)),
        height: String(Math.round(panel.api.height)),
      });
      setResize(true);
    },
  });
  return (
    <>
      <div className="v2-prototype-toolbar">
        <div>
          <span className="v2-preview-label">V2 foundations</span>
          <span className="v2-fixture-label">
            <Icon name="info" size={12} />
            Fixture preview · no runtime
          </span>
        </div>
        <div>
          <Select
            label="Prototype scenario"
            hideLabel
            value={state.scenario}
            onChange={(event) => {
              model.persist();
              const query = new URLSearchParams(location.search);
              query.set("scenario", event.target.value);
              location.search = query.toString();
            }}
          >
            {scenarios.map((scenario) => (
              <option key={scenario.id} value={scenario.id}>
                {scenario.title}
              </option>
            ))}
          </Select>
          <Select
            label="Theme"
            hideLabel
            value={theme.preference}
            onChange={(event) =>
              theme.setPreference(event.target.value as "system" | "light" | "dark")
            }
          >
            <option value="system">System theme</option>
            <option value="light">Light theme</option>
            <option value="dark">Dark theme</option>
          </Select>
          <Button tone="quiet" onClick={onGallery}>
            Components
          </Button>
        </div>
      </div>
      <header className="v2-workspace-top">
        <div className="v2-brand">
          <svg width="20" height="20" viewBox="0 0 20 20" aria-hidden="true">
            <path
              d="M3 15V5h5v10M8 5l5 5v5M13 10l4-4v9"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
            />
          </svg>
          <h1 aria-label="Agentic Software Factory">ASF</h1>
        </div>
        <div
          role="tablist"
          aria-label="Project fixtures"
          className="v2-project-tabs"
          onKeyDown={(event) => {
            if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
              event.preventDefault();
              switchProject(model.project === "atlas" ? "fieldnotes" : "atlas");
            }
          }}
        >
          <WorkspaceTab
            id="project-atlas"
            label="Atlas"
            selected={model.project === "atlas"}
            onSelect={() => {
              if (model.project !== "atlas") switchProject("atlas");
            }}
          />
          <WorkspaceTab
            id="project-fieldnotes"
            label="Fieldnotes"
            selected={model.project === "fieldnotes"}
            onSelect={() => {
              if (model.project !== "fieldnotes") switchProject("fieldnotes");
            }}
          />
        </div>
        <span className="v2-session-label">Session · Checkpoint recovery</span>
        <button type="button" className="v2-global-search" onClick={() => setSearch(true)}>
          <Icon name="search" size={14} />
          <span>Search workspace</span>
          <kbd>Ctrl K</kbd>
        </button>
        <div className="v2-top-actions">
          <Dropdown label="Views" icon="layout" entries={views} />
          <Tooltip content="Notifications">
            <IconButton
              tone="quiet"
              icon="bell"
              label={`Notifications (${state.notifications.length})`}
              onClick={() => setNotifications(true)}
            />
          </Tooltip>
          <IconButton
            tone="quiet"
            icon="settings"
            label="Workspace preferences"
            onClick={() => setSettings(true)}
          />
        </div>
      </header>
      <div className="v2-layout-bar">
        <Tabs
          label="Workspace presets"
          items={["Focus", "Code", "Agents", "Review"].map((label) => ({ id: label, label }))}
          value={state.preset}
          onChange={(value) => model.setPreset(value as Preset)}
        />
        <span className="v2-layout-hint">Drag tabs to arrange views</span>
        <div className="v2-layout-actions">
          <Tooltip content="Move active tab left">
            <IconButton
              label="Move active tab left"
              disabled={activeIndex <= 0}
              icon="chevron"
              tone="quiet"
              className="v2-flip-icon"
              onClick={() => model.moveActive("left")}
            />
          </Tooltip>
          <Tooltip content="Move active tab right">
            <IconButton
              label="Move active tab right"
              disabled={!activePanel || activeIndex >= activePanel.group.panels.length - 1}
              icon="chevron"
              tone="quiet"
              onClick={() => model.moveActive("right")}
            />
          </Tooltip>
          <Tooltip content="Split active editor">
            <IconButton
              label="Split active editor"
              icon="split"
              tone="quiet"
              onClick={() => model.splitActive()}
            />
          </Tooltip>
          <Tooltip content="Toggle explorer">
            <IconButton
              label="Toggle explorer"
              icon="collapse"
              tone="quiet"
              onClick={() => model.toggleView("explorer")}
            />
          </Tooltip>
          <Tooltip content="Toggle output">
            <IconButton
              label="Toggle output"
              icon="terminal"
              tone="quiet"
              onClick={() => model.toggleView("output")}
            />
          </Tooltip>
        </div>
      </div>
      {(state.issue || theme.issue) && (
        <InlineAlert title="Preview recovery" tone="warning">
          {state.issue || theme.issue}
          <Button tone="quiet" onClick={() => model.resetLayout()}>
            Restore preset
          </Button>
        </InlineAlert>
      )}
      <div className="v2-workspace-body">
        <nav className="v2-activity-rail" aria-label="Activity">
          <ActivityBarItem
            icon="files"
            label="Explorer"
            selected={state.activity === "explorer"}
            onClick={() => {
              model.update({ activity: "explorer" });
              model.showView("explorer");
            }}
          />
          <ActivityBarItem icon="search" label="Search workspace" onClick={() => setSearch(true)} />
          <ActivityBarItem
            icon="git"
            label="Source control"
            selected={state.activity === "git"}
            onClick={() => {
              model.update({ activity: "git" });
              model.setPreset("Review");
            }}
          />
          <ActivityBarItem
            icon="agents"
            label="Agent supervision"
            selected={state.activity === "agents"}
            onClick={() => {
              model.update({ activity: "agents" });
              model.showView("agents");
            }}
          />
          <div className="v2-activity-spacer" />
          <ActivityBarItem icon="settings" label="Preferences" onClick={() => setSettings(true)} />
        </nav>
        <DockSurface />
      </div>
      <footer className="v2-statusbar">
        <StatusBarItem icon="git">main · fixture</StatusBarItem>
        <StatusBarItem icon="checkpoint">cp-014</StatusBarItem>
        <DiagnosticsIndicator errors={0} warnings={0} />
        <span className="v2-statusbar-spacer" />
        <StatusBarItem onClick={() => setSettings(true)}>
          {compact ? "Compact" : "Default"} density
        </StatusBarItem>
        <LanguageLabel
          language={
            model.files.find((file) => file.path === state.selectedFile)?.language ?? "Rust"
          }
        />
        <StatusBarItem>UTF-8</StatusBarItem>
        <StatusBarItem icon="info">Browser preview only</StatusBarItem>
      </footer>
      <span className="ds-sr-only" role="status" aria-live="polite">
        {state.notice}
      </span>
      <SearchDialog open={search} onClose={() => setSearch(false)} />
      <Dialog
        open={notifications}
        onClose={() => setNotifications(false)}
        title="Notifications"
        description="Recorded fixture notices and local preview actions."
      >
        {state.notifications.length ? (
          state.notifications.map((item) => (
            <NotificationItem
              key={item.id}
              item={item}
              onDismiss={(id) => model.dismissNotification(id)}
            />
          ))
        ) : (
          <EmptyState
            title="No notifications"
            description="All preview notifications have been dismissed."
          />
        )}
      </Dialog>
      <Dialog
        open={settings}
        onClose={() => setSettings(false)}
        title="Workspace preferences"
        description="Appearance and density are saved locally for this preview."
        footer={
          <Button tone="primary" onClick={() => setSettings(false)}>
            Done
          </Button>
        }
      >
        <Select
          label="Appearance"
          value={theme.preference}
          onChange={(event) =>
            theme.setPreference(event.target.value as "system" | "light" | "dark")
          }
        >
          <option value="system">Use system theme</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </Select>
        <Toggle
          label="Compact interface density"
          checked={compact}
          onChange={(value) => {
            setCompact(value);
            try {
              localStorage.setItem("asf.v2.p02.density", value ? "compact" : "default");
            } catch {
              model.notify("Density applied for this session", "Browser storage is unavailable.");
            }
          }}
        />
        <p className="v2-fine-print">
          Inter for the interface. JetBrains Mono for code and technical values. Both fonts are
          served locally.
        </p>
        <div className="v2-shortcuts">
          <span>Search workspace</span>
          <kbd>Ctrl K</kbd>
          <span>Next panel</span>
          <kbd>F6</kbd>
          <span>Send a preview request</span>
          <kbd>Ctrl Enter</kbd>
        </div>
        <Button onClick={() => setReset(true)}>Restore preset layout</Button>
      </Dialog>
      <ConfirmationDialog
        open={reset}
        onClose={() => setReset(false)}
        title="Restore this preset layout?"
        description="Panels return to their preset positions. Local buffers and conversation drafts are retained."
        confirmLabel="Restore layout"
        onConfirm={() => {
          model.resetLayout();
          setSettings(false);
        }}
      />
      <Dialog
        open={resize}
        onClose={() => setResize(false)}
        title="Resize active panel"
        description="Set the requested group dimensions. Adjacent panels retain their minimum sizes. This also provides a keyboard alternative to dragging a divider."
        footer={
          <Button
            tone="primary"
            disabled={
              !Number.isFinite(Number(dimensions.width)) ||
              Number(dimensions.width) < 160 ||
              !Number.isFinite(Number(dimensions.height)) ||
              Number(dimensions.height) < 100
            }
            onClick={() => {
              model.api?.activePanel?.api.setSize({
                width: Math.min(window.innerWidth, Number(dimensions.width)),
                height: Math.min(window.innerHeight, Number(dimensions.height)),
              });
              model.update({ notice: "Active panel dimensions updated" });
              setResize(false);
            }}
          >
            Apply dimensions
          </Button>
        }
      >
        <Input
          label="Panel width"
          type="number"
          min={160}
          max={window.innerWidth}
          value={dimensions.width}
          onChange={(event) =>
            setDimensions((previous) => ({ ...previous, width: event.target.value }))
          }
        />
        <Input
          label="Panel height"
          type="number"
          min={100}
          max={window.innerHeight}
          value={dimensions.height}
          onChange={(event) =>
            setDimensions((previous) => ({ ...previous, height: event.target.value }))
          }
        />
      </Dialog>
    </>
  );
}

export function WorkspacePrototype() {
  const [model] = useState(() => new WorkspaceController());
  const [gallery, setGallery] = useState(
    new URLSearchParams(location.search).get("gallery") === "components"
  );
  useEffect(() => {
    window.__ASF_P02__ = { controller: model };
    return () => {
      delete window.__ASF_P02__;
    };
  }, [model]);
  return (
    <WorkspaceContext.Provider value={model}>
      <main className="asf-v2" data-preview="P02">
        {gallery ? (
          <ComponentGallery onClose={() => setGallery(false)} />
        ) : (
          <WorkspaceChrome onGallery={() => setGallery(true)} />
        )}
      </main>
    </WorkspaceContext.Provider>
  );
}
