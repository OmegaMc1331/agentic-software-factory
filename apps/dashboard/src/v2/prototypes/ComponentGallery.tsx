import { useState, type ReactNode } from "react";
import {
  ActivityBarItem,
  Breadcrumb,
  CompactStatus,
  ConfirmationPrompt,
  EmptyState,
  FileTreeItem,
  InlineAlert,
  LoadingIndicator,
  PanelHeader,
  PanelToolbar,
  ProgressRow,
  ResizableDivider,
  StatusBarItem,
  ViewErrorBoundary,
  WorkspaceTab,
} from "../design-system/components/Application";
import {
  Button,
  Checkbox,
  IconButton,
  Input,
  RadioControl,
  SearchField,
  Select,
  Tabs,
  Textarea,
  Toggle,
} from "../design-system/primitives/Controls";
import {
  ConfirmationDialog,
  ContextMenu,
  Dialog,
  Dropdown,
  Popover,
  Tooltip,
} from "../design-system/primitives/Overlays";
import {
  AgentIdentity,
  AgentStatus,
  AttentionNotification,
  CheckpointState,
  CodeChangeSummary,
  CurrentMissionStep,
  MissionStatus,
  OrchestratorMessage,
  ReferencedFile,
  ResultSummary,
  ReviewSummary,
  TestResult,
  UserMessage,
} from "../design-system/patterns/Agentic";
import {
  CodeEditorContainer,
  DebugToolbar,
  DiagnosticsIndicator,
  EditorTabs,
  FileBreadcrumb,
  GitModificationIndicator,
  LanguageLabel,
  TerminalHeader,
} from "../design-system/patterns/Code";
import { agents, fixtureFiles, messagesFor, missionFor, tests } from "./fixtures/data";
import { useTheme } from "../design-system/themes/theme";

function Section({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: ReactNode;
}) {
  return (
    <section className="v2-gallery-section">
      <header>
        <h2>{title}</h2>
        <p>{detail}</p>
      </header>
      <div className="v2-gallery-examples">{children}</div>
    </section>
  );
}
function FailureFixture({ fail }: { fail: boolean }) {
  if (fail) throw new Error("The fixture view was interrupted. Other panels are still available.");
  return <p>The fixture view is available.</p>;
}

export function ComponentGallery({ onClose }: { onClose(): void }) {
  const theme = useTheme();
  const [name, setName] = useState("Checkpoint recovery");
  const [search, setSearch] = useState("");
  const [checked, setChecked] = useState(true);
  const [radio, setRadio] = useState("project");
  const [toggle, setToggle] = useState(true);
  const [tab, setTab] = useState("overview");
  const [dialog, setDialog] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [notice, setNotice] = useState("Controls update only gallery state.");
  const [width, setWidth] = useState(210);
  const [failed, setFailed] = useState(false);
  const [prompt, setPrompt] = useState(true);
  const [channel, setChannel] = useState("Terminal");
  const [debug, setDebug] = useState<"paused" | "running" | "stopped">("paused");
  const [editorTab, setEditorTab] = useState("rust");
  const menu = [
    {
      id: "reference",
      label: "Add fixture reference",
      icon: "file" as const,
      action: () => setNotice("Fixture reference added"),
    },
    {
      id: "review",
      label: "Open fixture review",
      icon: "git" as const,
      action: () => setNotice("Review selected in gallery"),
    },
    {
      id: "publish",
      label: "Publish · connection required",
      icon: "link" as const,
      disabled: true,
      action: () => undefined,
    },
  ];
  const mission = missionFor("running");
  const messages = messagesFor("conversation");
  return (
    <div className="v2-gallery">
      <PanelHeader title="Component library" detail="Interactive states · same application tokens">
        <Select
          label="Gallery theme"
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
        <Button tone="quiet" onClick={onClose}>
          Return to workspace
        </Button>
      </PanelHeader>
      <div className="v2-gallery-scroll">
        <div className="v2-gallery-intro">
          <h1>Interface foundations</h1>
          <p>
            Reusable controls and domain presentations. All data and actions in this gallery are
            local fixtures.
          </p>
        </div>
        <Section title="Actions" detail="Default, hover, focus, pressed, disabled and busy states.">
          <div className="ds-inline-actions">
            <Button tone="primary" onClick={() => setNotice("Primary action selected")}>
              Primary action
            </Button>
            <Button onClick={() => setNotice("Secondary action selected")}>Secondary</Button>
            <Button tone="quiet" onClick={() => setNotice("Quiet action selected")}>
              Quiet
            </Button>
            <Button tone="danger" onClick={() => setConfirm(true)}>
              Remove fixture
            </Button>
            <Button disabled>Unavailable</Button>
            <Button loading>Saving preview</Button>
            <IconButton
              icon="plus"
              label="Add gallery item"
              onClick={() => setNotice("Gallery item added")}
            />
            <Tooltip content="Keyboard and pointer tooltip">
              <IconButton icon="info" tone="quiet" label="Control information" />
            </Tooltip>
          </div>
          <div className="v2-gallery-status" role="status">
            {notice}
          </div>
        </Section>
        <Section
          title="Fields"
          detail="Native input semantics, explicit labels, validation and disabled states."
        >
          <div className="v2-gallery-fields">
            <Input
              label="Session name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              error={!name.trim() ? "A session name is required." : undefined}
            />
            <Input label="Unavailable field" value="Managed by policy" disabled />
            <Select
              label="Permission scope"
              value={radio}
              onChange={(event) => setRadio(event.target.value)}
            >
              <option value="project">Project files</option>
              <option value="worktree">Current worktree</option>
            </Select>
            <Textarea
              label="Acceptance criteria"
              defaultValue="Keep the ownership guard explicit and preserve the existing error context."
            />
          </div>
          <SearchField label="Search gallery examples" value={search} onChange={setSearch} />
          <p className="v2-fine-print">
            {search ? `Local search value: ${search}` : "Search supports clear and Escape."}
          </p>
          <div className="ds-inline-actions">
            <Checkbox
              label="Include verified context"
              checked={checked}
              onChange={(event) => setChecked(event.target.checked)}
            />
            <Checkbox label="Inherited setting" checked disabled />
            <RadioControl
              label="Project"
              name="gallery-scope"
              checked={radio === "project"}
              onChange={() => setRadio("project")}
            />
            <RadioControl
              label="Worktree"
              name="gallery-scope"
              checked={radio === "worktree"}
              onChange={() => setRadio("worktree")}
            />
            <Toggle label="Show secondary details" checked={toggle} onChange={setToggle} />
          </div>
        </Section>
        <Section
          title="Overlays"
          detail="Portaled menus, native modal focus, Escape and focus return."
        >
          <div className="ds-inline-actions">
            <Dropdown label="Gallery menu" entries={menu} tone="neutral" />
            <ContextMenu label="Gallery context actions" entries={menu}>
              <Button>Right-click or Shift F10</Button>
            </ContextMenu>
            <Button onClick={() => setDialog(true)}>Open dialog</Button>
            <Button onClick={() => setConfirm(true)}>Confirm action</Button>
            <Popover label="Context budget" trigger="Context details">
              <p>Only relevant project entries should be included in a mission.</p>
              <Checkbox label="Include architecture decisions" defaultChecked />
            </Popover>
          </div>
          <Tabs
            label="Gallery tabs"
            items={[
              { id: "overview", label: "Overview", controls: "gallery-tab-content" },
              { id: "details", label: "Details", controls: "gallery-tab-content" },
              { id: "unavailable", label: "Unavailable", disabled: true },
            ]}
            value={tab}
            onChange={setTab}
          />
          <div id="gallery-tab-content" role="tabpanel" aria-label={tab}>
            {tab === "overview"
              ? "Overview selected. Arrow keys switch tabs."
              : "Details selected. Disabled tabs are skipped."}
          </div>
        </Section>
        <Section
          title="Workspace vocabulary"
          detail="Compact navigation, panel hierarchy, paths and real resizer events."
        >
          <PanelHeader title="Explorer" detail="Current fixture">
            <IconButton
              icon="more"
              label="Gallery panel action"
              tone="quiet"
              onClick={() => setNotice("Panel action selected")}
            />
          </PanelHeader>
          <PanelToolbar label="Gallery panel toolbar">
            <ActivityBarItem
              icon="files"
              label="Files"
              selected
              onClick={() => setNotice("Files selected")}
            />
            <ActivityBarItem
              icon="agents"
              label="Agents"
              onClick={() => setNotice("Agents selected")}
            />
            <Button tone="quiet" icon="split" onClick={() => setNotice("Split action selected")}>
              Split view
            </Button>
          </PanelToolbar>
          <div role="tablist" aria-label="Workspace tab example">
            <WorkspaceTab
              id="gallery-workspace-tab"
              label="checkpoints.rs"
              selected
              dirty
              onSelect={() => setNotice("Workspace tab selected")}
              onClose={() => setNotice("Close requested for fixture tab")}
            />
          </div>
          <Breadcrumb
            items={[
              { label: "Atlas", onClick: () => setNotice("Project breadcrumb selected") },
              { label: "src" },
              { label: "runtime" },
              { label: "checkpoint_ownership_reconciliation.rs" },
            ]}
          />
          <div role="tree" aria-label="Gallery tree">
            <FileTreeItem
              path="src/runtime/checkpoints.rs"
              label="checkpoints.rs"
              depth={2}
              selected
              onSelect={() => setNotice("Fixture tree item selected")}
            />
          </div>
          <div className="v2-gallery-divider">
            <div id="gallery-resized-panel" style={{ width }}>
              Resizable fixture: {Math.round(width)} px
            </div>
            <ResizableDivider
              label="Resize gallery panel"
              controls="gallery-resized-panel"
              value={width}
              min={140}
              max={420}
              onChange={setWidth}
            />
            <span>Drag or use arrow keys</span>
          </div>
          <div className="v2-gallery-statusbar">
            <StatusBarItem icon="git">main · fixture</StatusBarItem>
            <StatusBarItem icon="settings" onClick={() => setNotice("Status action selected")}>
              Preferences
            </StatusBarItem>
          </div>
        </Section>
        <Section
          title="Feedback and recovery"
          detail="States include text as well as color; no simulated runtime activity."
        >
          <div className="ds-inline-actions">
            {(["idle", "running", "completed", "failed", "attention", "suspended"] as const).map(
              (status) => (
                <CompactStatus key={status} status={status} />
              )
            )}
            <LoadingIndicator label="Loading fixture state" />
          </div>
          <InlineAlert tone="danger" title="The operation could not be verified">
            Keep the last verified state and inspect the error before retrying.
          </InlineAlert>
          <InlineAlert tone="success" title="Fixture accepted">
            This state is explicitly local to the preview.
          </InlineAlert>
          <EmptyState
            title="No diagnostics available"
            description="Connect a language server in the application to receive diagnostics."
            action={
              <Button onClick={() => setNotice("Availability explanation selected")}>
                View requirements
              </Button>
            }
          />
          {prompt && (
            <ConfirmationPrompt
              title="Use the verified checkpoint?"
              description="The selected checkpoint preserves the previous attempt and its changes."
              confirmLabel="Use fixture checkpoint"
              onConfirm={() => {
                setPrompt(false);
                setNotice("Fixture checkpoint selected");
              }}
              onCancel={() => setPrompt(false)}
            />
          )}
          <Button onClick={() => setFailed(true)}>Show failed view</Button>
          <ViewErrorBoundary onRetry={() => setFailed(false)}>
            <FailureFixture fail={failed} />
          </ViewErrorBoundary>
        </Section>
        <Section
          title="Conversation and results"
          detail="Typed messages, references and review entry points."
        >
          <UserMessage message={messages[0]} />
          <OrchestratorMessage
            message={messages[1]}
            onOpenFile={(path) => setNotice(`Selected ${path}`)}
            onReview={() => setNotice("Fixture review selected")}
          />
          <ReferencedFile file={fixtureFiles[0]} onOpen={(path) => setNotice(`Selected ${path}`)} />
          <ResultSummary
            provenance="fixture"
            title="Ownership validation ready"
            files={4}
            tests={9}
            onReview={() => setNotice("Review fixture selected")}
          />
          <CodeChangeSummary files={fixtureFiles.slice(0, 4)} />
        </Section>
        <Section
          title="Mission and agents"
          detail="Progress comes from task states in the supplied domain record."
        >
          <div className="v2-gallery-fields">
            <div>
              <MissionStatus mission={mission} />
              <CurrentMissionStep mission={mission} />
              <ol className="v2-task-list">
                {mission.steps.slice(0, 3).map((step) => (
                  <ProgressRow
                    key={step.id}
                    title={step.title}
                    state={step.status}
                    detail={step.detail}
                  />
                ))}
              </ol>
            </div>
            <div>
              <AgentIdentity agent={agents[0]} />
              <AgentStatus agent={agents[0]} />
              <CheckpointState checkpoint={mission.checkpoint} />
              <AttentionNotification
                title="Agent unavailable"
                detail="Choose a replacement after verifying the checkpoint."
              />
            </div>
          </div>
          <ReviewSummary
            status="pending"
            summary="The review checks the supplied evidence and preserves limitations."
          >
            <TestResult test={tests[0]} />
          </ReviewSummary>
        </Section>
        <Section
          title="Code and tools"
          detail="Presentation contracts; this gallery has no LSP, DAP or shell connection."
        >
          <CodeEditorContainer label="Code container example">
            <EditorTabs
              items={[
                { id: "rust", label: "checkpoints.rs" },
                { id: "typescript", label: "session.ts" },
              ]}
              value={editorTab}
              onChange={setEditorTab}
            />
            <FileBreadcrumb
              path={editorTab === "rust" ? fixtureFiles[0].path : fixtureFiles[2].path}
            />
            <pre className="v2-code-sample">
              {editorTab === "rust" ? fixtureFiles[0].text : fixtureFiles[2].text}
            </pre>
          </CodeEditorContainer>
          <div className="ds-inline-actions">
            <LanguageLabel language={editorTab === "rust" ? "Rust" : "TypeScript"} />
            <DiagnosticsIndicator errors={1} warnings={2} />
            <GitModificationIndicator status="modified" origin="agent" />
            <span className="v2-fine-print">Fixture indicators</span>
          </div>
          <DebugToolbar
            provenance="fixture"
            state={debug}
            onAction={(action) => {
              setDebug(
                action === "continue" ? "running" : action === "stop" ? "stopped" : "paused"
              );
              setNotice(`Debug fixture: ${action}`);
            }}
          />
          {debug === "stopped" && (
            <Button onClick={() => setDebug("paused")}>Reset debug fixture</Button>
          )}
          <TerminalHeader
            panelId="fixture-output"
            provenance="fixture"
            channel={channel}
            onChange={setChannel}
            onClear={() => setNotice("Output fixture cleared")}
          />
          <div id="fixture-output" role="tabpanel" aria-label={channel} className="v2-code-sample">
            {channel} · recorded fixture
          </div>
        </Section>
      </div>
      <Dialog
        open={dialog}
        onClose={() => setDialog(false)}
        title="Edit preview preferences"
        description="These controls demonstrate focus, validation and a local draft."
        footer={
          <>
            <Button onClick={() => setDialog(false)}>Cancel</Button>
            <Button
              tone="primary"
              disabled={!name.trim()}
              onClick={() => {
                setDialog(false);
                setNotice("Gallery preferences applied");
              }}
            >
              Apply
            </Button>
          </>
        }
      >
        <Input
          label="Dialog session name"
          value={name}
          onChange={(event) => setName(event.target.value)}
          error={!name.trim() ? "A name is required." : undefined}
        />
        <Select
          label="Dialog scope"
          value={radio}
          onChange={(event) => setRadio(event.target.value)}
        >
          <option value="project">Project</option>
          <option value="worktree">Worktree</option>
        </Select>
      </Dialog>
      <ConfirmationDialog
        open={confirm}
        onClose={() => setConfirm(false)}
        onConfirm={() => setNotice("Fixture removal confirmed; no project data was changed")}
        title="Remove this gallery fixture?"
        description="Only this preview selection changes. Project files and stored missions are unaffected."
        confirmLabel="Remove fixture"
        danger
      />
    </div>
  );
}
