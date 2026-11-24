import { StateEffect, StateField, RangeSet, type Extension } from "@codemirror/state";
import { Decoration, EditorView, GutterMarker, ViewPlugin, gutter } from "@codemirror/view";
import type { FileRef } from "../files/types";
import { fileKey } from "../files/types";
import { ideRequest } from "./api";
import { FactoryApiError } from "../../api";
import type { DebugInfo, EventPage } from "./types";

export interface Breakpoint {
  file: FileRef;
  line: number;
  condition?: string;
}
export interface StackFrame {
  id: number;
  name: string;
  line: number;
  column: number;
  asfFile?: FileRef | null;
}
export interface Variable {
  name: string;
  value: string;
  type?: string;
  variablesReference: number;
  scope?: string;
}
export interface DebugState {
  info: DebugInfo;
  cursor: number;
  status: string;
  threadId?: number;
  threads: Array<{ id: number; name: string }>;
  activeFrameId?: number;
  frames: StackFrame[];
  variables: Variable[];
  output: string;
  error: string;
  stopped: boolean;
  inspection: number;
}
export class DebugManager {
  private listeners = new Set<() => void>();
  private revision = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private disposed = false;
  private polling = false;
  readonly sessions = new Map<string, DebugState>();
  breakpoints: Breakpoint[] = [];
  private locations = new Map<string, { file: FileRef; line: number }>();
  executionLine(file: FileRef) {
    return [...this.locations.values()]
      .reverse()
      .find((item) => fileKey(item.file) === fileKey(file))?.line;
  }
  persistenceError = "";
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly snapshot = () => this.revision;
  constructor() {
    try {
      const saved = JSON.parse(
        localStorage.getItem("asf.v2.breakpoints.1") ?? "[]"
      ) as Breakpoint[];
      this.breakpoints = saved
        .filter(
          (p) =>
            p.file &&
            /^p_[0-9a-f]{32}$/.test(p.file.project_id) &&
            /^r_[0-9a-f]{32}$/.test(p.file.root_id) &&
            typeof p.file.path === "string" &&
            p.file.path.length < 2048 &&
            Number.isSafeInteger(p.line) &&
            p.line > 0
        )
        .slice(0, 500);
    } catch {
      this.breakpoints = [];
    }
  }
  private publish() {
    this.revision++;
    for (const listener of this.listeners) listener();
  }
  private persist() {
    try {
      localStorage.setItem("asf.v2.breakpoints.1", JSON.stringify(this.breakpoints));
    } catch {
      this.persistenceError =
        "Breakpoint persistence is unavailable. Current breakpoints remain active for this window.";
    }
    this.publish();
  }
  toggle(file: FileRef, line: number) {
    const index = this.breakpoints.findIndex(
      (p) => fileKey(p.file) === fileKey(file) && p.line === line
    );
    if (index < 0) this.breakpoints.push({ file, line });
    else this.breakpoints.splice(index, 1);
    this.persist();
    void this.syncBreakpoints(file);
  }
  condition(point: Breakpoint, condition: string) {
    point.condition = condition || undefined;
    this.persist();
    void this.syncBreakpoints(point.file);
  }
  private async syncBreakpoints(file: FileRef) {
    for (const [key, state] of this.sessions)
      if (key.startsWith(`${file.project_id}:`) && state.info.root_id === file.root_id)
        try {
          await ideRequest(file.project_id, `dap/${state.info.id}/breakpoints`, {
            file,
            breakpoints: this.breakpoints
              .filter((p) => fileKey(p.file) === fileKey(file))
              .map((p) => ({ line: p.line, condition: p.condition })),
          });
        } catch (error) {
          state.error = String(error);
          this.publish();
        }
  }
  adopt(project: string, info: DebugInfo) {
    const key = `${project}:${info.id}`;
    if (this.sessions.has(key)) return;
    this.sessions.set(key, {
      info,
      cursor: 0,
      status: "Running",
      threads: [],
      frames: [],
      variables: [],
      output: "",
      error: "",
      stopped: false,
      inspection: 0,
    });
    if (!this.timer) this.poll();
    this.publish();
  }
  async start(project: string, configuration: string) {
    const byFile = new Map<
      string,
      { file: FileRef; breakpoints: Array<{ line: number; condition?: string }> }
    >();
    for (const point of this.breakpoints.filter((p) => p.file.project_id === project)) {
      const key = fileKey(point.file);
      const current = byFile.get(key) ?? { file: point.file, breakpoints: [] };
      current.breakpoints.push({ line: point.line, condition: point.condition });
      byFile.set(key, current);
    }
    const info = await ideRequest<DebugInfo>(project, "dap/start", {
      configuration_id: configuration,
      breakpoints: [...byFile.values()],
      confirmed: true,
    });
    this.adopt(project, info);
    return info;
  }
  async command(
    project: string,
    id: string,
    command: string,
    args: Record<string, unknown> = {},
    confirmed = false
  ) {
    const result = await ideRequest<unknown>(project, `dap/${id}/request`, {
      command,
      arguments: args,
      confirmed,
    });
    if (["continue", "next", "stepIn", "stepOut"].includes(command)) {
      const state = this.sessions.get(`${project}:${id}`);
      if (state) state.inspection++;
      this.locations.delete(`${project}:${id}`);
      this.publish();
    }
    return result;
  }
  async selectFrame(project: string, state: DebugState, frame: StackFrame) {
    const key = `${project}:${state.info.id}`;
    const inspection = ++state.inspection;
    state.activeFrameId = frame.id;
    if (frame.asfFile) this.locations.set(key, { file: frame.asfFile, line: frame.line });
    else this.locations.delete(key);
    const scopes = (await this.command(project, state.info.id, "scopes", {
      frameId: frame.id,
    })) as { scopes: Array<{ variablesReference: number; name?: string }> };
    if (state.inspection !== inspection || this.sessions.get(key) !== state) return;
    const variables: Variable[] = [];
    for (const scope of scopes.scopes.slice(0, 3)) {
      const result = (await this.command(project, state.info.id, "variables", {
        variablesReference: scope.variablesReference,
      })) as { variables: Variable[] };
      if (state.inspection !== inspection || this.sessions.get(key) !== state) return;
      variables.push(
        ...result.variables
          .slice(0, 200)
          .map((variable) => ({ ...variable, scope: scope.name ?? "Scope" }))
      );
    }
    state.variables = variables;
    this.publish();
  }
  async stop(project: string, id: string) {
    try {
      await ideRequest(project, `dap/${id}/stop`, {});
    } catch (error) {
      if (!(error instanceof FactoryApiError && error.status === 404)) throw error;
    }
    this.sessions.delete(`${project}:${id}`);
    this.locations.delete(`${project}:${id}`);
    this.publish();
  }
  async selectThread(project: string, state: DebugState, id: number) {
    const inspection = ++state.inspection;
    state.threadId = id;
    const result = (await this.command(project, state.info.id, "stackTrace", { threadId: id })) as {
      stackFrames: StackFrame[];
    };
    if (
      state.inspection !== inspection ||
      this.sessions.get(`${project}:${state.info.id}`) !== state
    )
      return;
    state.frames = result.stackFrames;
    if (state.frames[0]) await this.selectFrame(project, state, state.frames[0]);
    this.publish();
  }
  private poll() {
    if (this.disposed || this.polling) return;
    this.polling = true;
    this.timer = undefined;
    let mutated = false;
    void Promise.all(
      [...this.sessions].map(async ([key, state]) => {
        if (state.stopped) return;
        const project = key.split(":")[0];
        try {
          const page = await ideRequest<EventPage>(
            project,
            `dap/${state.info.id}/events?cursor=${state.cursor}`
          );
          if (page.events.length || page.gap || !page.alive) mutated = true;
          state.cursor = page.cursor;
          if (page.gap) state.error = "Debug output was truncated to the bounded event history.";
          for (const event of page.events) {
            const message = event.message;
            const body = message.body as Record<string, unknown> | undefined;
            if (message.event === "output" && body?.category !== "telemetry")
              state.output = (state.output + String(body?.output ?? "")).slice(-100000);
            if (message.event === "continued") {
              state.status = "Running";
              state.frames = [];
              state.variables = [];
              state.inspection++;
              this.locations.delete(key);
            }
            if (message.event === "stopped") {
              state.status = `Stopped · ${String(body?.reason ?? "breakpoint")}`;
              const threads = (await this.command(project, state.info.id, "threads")) as {
                threads: Array<{ id: number; name: string }>;
              };
              state.threads = threads.threads;
              state.threadId =
                typeof body?.threadId === "number" ? body.threadId : state.threads[0]?.id;
              const result = (await this.command(project, state.info.id, "stackTrace", {
                threadId: state.threadId,
              })) as { stackFrames: StackFrame[] };
              state.frames = result.stackFrames;
              if (state.frames[0]) await this.selectFrame(project, state, state.frames[0]);
            }
            if (
              ["terminated", "exited"].includes(String(message.event)) ||
              message.type === "asf_exited"
            ) {
              state.status = "Terminated";
              state.stopped = true;
              state.inspection++;
              this.locations.delete(key);
            }
          }
          if (!page.alive) {
            state.stopped = true;
            state.status = "Adapter exited";
            state.inspection++;
            this.locations.delete(key);
          }
        } catch (error) {
          if (error instanceof FactoryApiError && error.status === 404) {
            state.stopped = true;
            state.status = "Session unavailable";
            state.inspection++;
            this.locations.delete(key);
          }
          if (state.error !== String(error)) {
            state.error = String(error);
            mutated = true;
          }
        }
      })
    ).finally(() => {
      this.polling = false;
      if (!this.disposed) {
        if (mutated) this.publish();
        if ([...this.sessions.values()].some((state) => !state.stopped))
          this.timer = setTimeout(() => this.poll(), 400);
      }
    });
  }
  dispose() {
    this.disposed = true;
    clearTimeout(this.timer);
    this.listeners.clear();
    this.sessions.clear();
    this.locations.clear();
  }
}

class BreakpointMarker extends GutterMarker {
  toDOM() {
    const node = document.createElement("span");
    node.className = "ide-breakpoint";
    node.textContent = "●";
    node.setAttribute("aria-label", "Breakpoint");
    return node;
  }
}
const marker = new BreakpointMarker();
const debugUpdate = StateEffect.define<{ lines: number[]; active?: number }>();
const debugState = StateField.define({
  create: () => ({ lines: [] as number[], active: undefined as number | undefined }),
  update(value, transaction) {
    for (const effect of transaction.effects)
      if (effect.is(debugUpdate))
        value = { lines: effect.value.lines, active: effect.value.active };
    return value;
  },
});
export function debugExtension(manager: DebugManager, file: FileRef): Extension {
  return [
    debugState,
    gutter({
      class: "ide-breakpoint-gutter",
      markers: (view) =>
        RangeSet.of(
          view.state
            .field(debugState)
            .lines.filter((line) => line <= view.state.doc.lines)
            .map((line) => marker.range(view.state.doc.line(line).from))
        ),
      initialSpacer: () => marker,
      domEventHandlers: {
        click: (view, line) => {
          manager.toggle(file, view.state.doc.lineAt(line.from).number);
          return true;
        },
      },
    }),
    EditorView.decorations.compute([debugState], (state) => {
      const active = state.field(debugState).active;
      return active && active <= state.doc.lines
        ? Decoration.set([
            Decoration.line({ class: "ide-execution-line" }).range(state.doc.line(active).from),
          ])
        : Decoration.none;
    }),
    ViewPlugin.define((view) => {
      let active = true;
      const update = () => {
        if (active)
          view.dispatch({
            effects: debugUpdate.of({
              lines: manager.breakpoints
                .filter((p) => fileKey(p.file) === fileKey(file))
                .map((p) => p.line)
                .sort((a, b) => a - b),
              active: manager.executionLine(file),
            }),
          });
      };
      queueMicrotask(update);
      const unsubscribe = manager.subscribe(update);
      return {
        destroy: () => {
          active = false;
          unsubscribe();
        },
      };
    }),
  ];
}
