import type { ReactNode } from "react";
import { Breadcrumb, PanelToolbar } from "../components/Application";
import { Button, IconButton, Tabs, type TabOption } from "../primitives/Controls";
import { Icon } from "../icons/Icon";

export function CodeEditorContainer({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section className="ds-editor-container" aria-label={label}>
      {children}
    </section>
  );
}

export function EditorTabs({
  items,
  value,
  onChange,
}: {
  items: TabOption[];
  value: string;
  onChange(id: string): void;
}) {
  return <Tabs label="Editor documents" items={items} value={value} onChange={onChange} />;
}

export function DiffToolbar({
  mode,
  onModeChange,
  onPrevious,
  onNext,
  provenance,
}: {
  mode: "split" | "unified";
  onModeChange(mode: "split" | "unified"): void;
  onPrevious(): void;
  onNext(): void;
  provenance: "fixture" | "runtime";
}) {
  return (
    <PanelToolbar label="Diff actions">
      <span className="ds-meta-label">{provenance === "fixture" ? "Fixture diff" : "Diff"}</span>
      <Button
        tone="quiet"
        aria-pressed={mode === "split"}
        onClick={() => onModeChange(mode === "split" ? "unified" : "split")}
      >
        {mode === "split" ? "Side by side" : "Unified"}
      </Button>
      <IconButton tone="quiet" icon="arrow" label="Previous change" onClick={onPrevious} />
      <IconButton tone="quiet" icon="down" label="Next change" onClick={onNext} />
    </PanelToolbar>
  );
}

export function FileBreadcrumb({ path }: { path: string }) {
  return <Breadcrumb items={path.split("/").map((label) => ({ label }))} />;
}

export function LanguageLabel({ language }: { language: string }) {
  return <span className="ds-language-label">{language}</span>;
}

export function DiagnosticsIndicator({ errors, warnings }: { errors: number; warnings: number }) {
  return (
    <span
      role="img"
      className="ds-diagnostics"
      aria-label={`${errors} errors, ${warnings} warnings`}
    >
      <Icon name="error" size={12} />
      {errors}
      <Icon name="warning" size={12} />
      {warnings}
    </span>
  );
}

export function GitModificationIndicator({
  status,
  origin,
}: {
  status: "modified" | "added" | "accepted";
  origin: "manual" | "agent";
}) {
  return (
    <span
      className={`ds-git-indicator ds-git-indicator--${status}`}
      role="img"
      title={`${status}, ${origin} changes`}
      aria-label={`${status}, ${origin} changes`}
    >
      {status === "modified" ? "M" : status === "added" ? "A" : "✓"}
      <span className="ds-sr-only"> {origin}</span>
    </span>
  );
}

export function TerminalHeader({
  channel,
  onChange,
  onClear,
  panelId,
  provenance,
}: {
  channel: string;
  onChange(value: string): void;
  onClear(): void;
  panelId: string;
  provenance: "fixture" | "runtime";
}) {
  return (
    <div className="ds-terminal-header">
      <Tabs
        label="Output channels"
        items={["Terminal", "Diagnostics", "Tests", "Logs"].map((label) => ({
          id: label,
          label,
          controls: panelId,
        }))}
        value={channel}
        onChange={onChange}
      />
      <IconButton
        label={provenance === "fixture" ? "Clear fixture output" : "Clear output"}
        tone="quiet"
        icon="clear"
        onClick={onClear}
      />
    </div>
  );
}

export function DebugToolbar({
  state,
  onAction,
  provenance,
}: {
  state: "paused" | "running" | "stopped";
  onAction(action: "continue" | "pause" | "step" | "stop"): void;
  provenance: "fixture" | "runtime";
}) {
  return (
    <PanelToolbar label={provenance === "fixture" ? "Debug preview actions" : "Debug actions"}>
      <IconButton
        label={
          state === "running"
            ? provenance === "fixture"
              ? "Pause debug fixture"
              : "Pause debugging"
            : provenance === "fixture"
              ? "Continue debug fixture"
              : "Continue debugging"
        }
        tone="quiet"
        icon={state === "running" ? "pause" : "play"}
        disabled={state === "stopped"}
        onClick={() => onAction(state === "running" ? "pause" : "continue")}
      />
      <IconButton
        label={provenance === "fixture" ? "Step debug fixture" : "Step debugging"}
        tone="quiet"
        icon="step"
        disabled={state !== "paused"}
        onClick={() => onAction("step")}
      />
      <IconButton
        label={provenance === "fixture" ? "Stop debug fixture" : "Stop debugging"}
        tone="quiet"
        icon="stop"
        disabled={state === "stopped"}
        onClick={() => onAction("stop")}
      />
      <span className="ds-meta-label">
        {state}
        {provenance === "fixture" ? " · fixture" : ""}
      </span>
    </PanelToolbar>
  );
}
