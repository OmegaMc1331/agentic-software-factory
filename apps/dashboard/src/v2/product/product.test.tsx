import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { SettingsPanel } from "./SettingsPanel";
import { defaultPreferences } from "./preferences";
import { NotificationCenter, NotificationSignals } from "./Notifications";
import { CommandPalette } from "./CommandPalette";
import type { WorkspaceModel } from "../workspace/production/controller";

const mocks = vi.hoisted(() => ({ request: vi.fn(), refresh: vi.fn(), read: vi.fn() }));
vi.mock("../../api", () => ({ request: mocks.request }));
vi.mock("../workspace/production/data", () => ({
  useData: (p: string, path: string) => mocks.read(p, path),
  refreshData: (p: string, path: string) => mocks.refresh(p, path),
  acknowledgeData: (p: string, path: string, value: unknown) => mocks.refresh(p, path, value),
}));
const project = "p_" + "a".repeat(32);
const model = {
  updateView: vi.fn(),
  navigateResource: vi.fn().mockResolvedValue(undefined),
} as unknown as WorkspaceModel;
beforeEach(() => {
  vi.clearAllMocks();
  mocks.read.mockImplementation((p: string) => ({
    value: p ? {} : { revision: 3, preferences: { ...defaultPreferences } },
    loading: false,
  }));
  mocks.refresh.mockResolvedValue(undefined);
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
describe("command palette keyboard entry", () => {
  const originalShow = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "showModal");
  const originalClose = Object.getOwnPropertyDescriptor(HTMLDialogElement.prototype, "close");
  beforeAll(() => {
    HTMLDialogElement.prototype.showModal = function () {
      this.setAttribute("open", "");
    };
    HTMLDialogElement.prototype.close = function () {
      this.removeAttribute("open");
    };
  });
  afterAll(() => {
    for (const [name, descriptor] of [
      ["showModal", originalShow],
      ["close", originalClose],
    ] as const) {
      if (descriptor) Object.defineProperty(HTMLDialogElement.prototype, name, descriptor);
      else Reflect.deleteProperty(HTMLDialogElement.prototype, name);
    }
  });
  it("focuses search after the native dialog opens and executes the selected command once", async () => {
    const run = vi.fn();
    const close = vi.fn();
    render(
      <CommandPalette
        open
        onClose={close}
        onError={vi.fn()}
        commands={[
          {
            id: "settings",
            title: "Open Settings",
            category: "Application",
            description: "Preferences",
            run,
          },
        ]}
      />
    );
    const search = screen.getByLabelText("Search commands");
    await waitFor(() => expect(document.activeElement).toBe(search));
    fireEvent.change(search, { target: { value: "settings" } });
    fireEvent.keyDown(search, { key: "Enter" });
    expect(close).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1));
  });
});
describe("centralized product settings", () => {
  it("searches a real category and focuses its actual editor control", async () => {
    render(<SettingsPanel model={model} />);
    fireEvent.change(screen.getByLabelText("Search settings"), {
      target: { value: "indentation" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    await waitFor(() =>
      expect(document.activeElement).toBe(screen.getByLabelText("Editor font size"))
    );
    expect(screen.getByLabelText("Indentation spaces")).toBeTruthy();
  });
  it("saves application preferences with their original backend revision", async () => {
    mocks.request.mockResolvedValue({
      revision: 4,
      preferences: { ...defaultPreferences, editor_font_size: 16 },
    });
    render(<SettingsPanel model={model} />);
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    fireEvent.change(screen.getByLabelText("Editor font size"), { target: { value: "16" } });
    fireEvent.click(screen.getByRole("button", { name: "Save application preferences" }));
    await screen.findByText("Application preferences saved by the server.");
    expect(JSON.parse(mocks.request.mock.calls[0][1].body)).toMatchObject({
      revision: 3,
      preferences: { editor_font_size: 16 },
    });
    expect(mocks.request.mock.calls[0][1].headers).toEqual({ "Content-Type": "application/json" });
    expect(mocks.refresh).toHaveBeenCalledWith(
      "",
      "/application/preferences",
      expect.objectContaining({ revision: 4 })
    );
  });
  it("retains user edits on a rejected concurrent write", async () => {
    mocks.request.mockRejectedValue(new Error("stale_preferences"));
    render(<SettingsPanel model={model} />);
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    fireEvent.change(screen.getByLabelText("Editor font size"), { target: { value: "17" } });
    fireEvent.click(screen.getByRole("button", { name: "Save application preferences" }));
    await screen.findByText(/stale_preferences/);
    expect((screen.getByLabelText("Editor font size") as HTMLInputElement).value).toBe("17");
    expect(screen.queryByText("Application preferences saved by the server.")).toBeNull();
  });
  it("restores keyboard focus after a successful save removes the focused action", async () => {
    let finish!: (value: unknown) => void;
    mocks.request.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(<SettingsPanel model={model} />);
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    const field = screen.getByLabelText<HTMLInputElement>("Editor font size");
    await waitFor(() => expect(document.activeElement).toBe(field));
    fireEvent.change(field, { target: { value: "16" } });
    const save = screen.getByRole("button", { name: "Save application preferences" });
    save.focus();
    fireEvent.click(save);
    expect(field.disabled).toBe(true);
    document.body.tabIndex = -1;
    document.body.focus();
    document.body.removeAttribute("tabindex");
    expect(document.activeElement).toBe(document.body);
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(performance.now());
      return 1;
    });
    await act(async () => {
      finish({ revision: 4, preferences: { ...defaultPreferences, editor_font_size: 16 } });
    });
    await screen.findByText("Application preferences saved by the server.");
    expect(save.isConnected).toBe(false);
    expect(field.disabled).toBe(false);
    await waitFor(() => expect(document.activeElement).toBe(field));
  });
  it("preserves focus moved outside Settings while a save is pending", async () => {
    let finish!: (value: unknown) => void;
    mocks.request.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    render(
      <>
        <button>Other workspace action</button>
        <SettingsPanel model={model} />
      </>
    );
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    const field = screen.getByLabelText("Editor font size");
    await waitFor(() => expect(document.activeElement).toBe(field));
    fireEvent.change(field, { target: { value: "16" } });
    const save = screen.getByRole("button", { name: "Save application preferences" });
    save.focus();
    fireEvent.click(save);
    const other = screen.getByRole("button", { name: "Other workspace action" });
    other.focus();
    finish({ revision: 4, preferences: { ...defaultPreferences, editor_font_size: 16 } });
    await screen.findByText("Application preferences saved by the server.");
    await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    expect(document.activeElement).toBe(other);
  });
  it("returns focus after cancelling edits without sending a mutation", async () => {
    render(<SettingsPanel model={model} />);
    fireEvent.click(screen.getByRole("button", { name: /Editor Application/ }));
    const field = screen.getByLabelText("Editor font size") as HTMLInputElement;
    await waitFor(() => expect(document.activeElement).toBe(field));
    fireEvent.change(field, { target: { value: "16" } });
    const cancel = screen.getByRole("button", { name: "Cancel preference edits" });
    cancel.focus();
    fireEvent.click(cancel);
    await waitFor(() => expect(document.activeElement).toBe(field));
    expect(field.value).toBe(String(defaultPreferences.editor_font_size));
    expect(cancel.isConnected).toBe(false);
    expect(mocks.request).not.toHaveBeenCalled();
  });
});
describe("notification presentation uses persisted project identity", () => {
  it("sends only generic OS content after a new real-feed identity and never on replay", () => {
    const os = vi.fn();
    class BrowserNotification {
      static permission = "granted";
      constructor(title: string, options: NotificationOptions) {
        os(title, options);
      }
    }
    vi.stubGlobal("Notification", BrowserNotification);
    let id = 1;
    mocks.read.mockImplementation((p: string) => ({
      value: p
        ? {
            project_id: project,
            unread: 1,
            items: [
              {
                id,
                title: "Private objective",
                message: "Private agent output",
                read_at: null,
                dismissed_at: null,
              },
            ],
          }
        : { revision: 1, preferences: { ...defaultPreferences, browser_notifications: true } },
    }));
    const props = { projectId: project, onUnread: vi.fn(), onArrival: vi.fn() };
    const rendered = render(<NotificationSignals {...props} />);
    expect(os).not.toHaveBeenCalled();
    id = 2;
    rendered.rerender(<NotificationSignals {...props} />);
    expect(os).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(os.mock.calls)).not.toContain("Private");
    rendered.rerender(<NotificationSignals {...props} />);
    expect(os).toHaveBeenCalledTimes(1);
  });
  it("keeps in-app arrivals functional when browser permission is denied", () => {
    const os = vi.fn();
    class BrowserNotification {
      static permission = "denied";
      constructor() {
        os();
      }
    }
    vi.stubGlobal("Notification", BrowserNotification);
    let id = 1;
    mocks.read.mockImplementation((p: string) => ({
      value: p
        ? { project_id: project, unread: id, items: [{ id, read_at: null, dismissed_at: null }] }
        : { revision: 1, preferences: { ...defaultPreferences, browser_notifications: true } },
    }));
    const props = { projectId: project, onUnread: vi.fn(), onArrival: vi.fn() };
    const rendered = render(<NotificationSignals {...props} />);
    id = 2;
    rendered.rerender(<NotificationSignals {...props} />);
    expect(props.onArrival).toHaveBeenCalledTimes(1);
    expect(props.onUnread).toHaveBeenLastCalledWith(project, 2);
    expect(os).not.toHaveBeenCalled();
  });
  it("navigates a real scoped record and acknowledges its read state", async () => {
    mocks.read.mockImplementation(() => ({
      value: {
        project_id: project,
        unread: 1,
        items: [
          {
            id: 9,
            revision: 2,
            source_event_id: "run:3:completed",
            event_type: "mission.completed",
            category: "mission",
            severity: "success",
            title: "Mission completed",
            message: "Recorded outcome",
            run_id: 3,
            target: "mission",
            created_at: "2026-11-15T10:00:00Z",
            read_at: null,
            dismissed_at: null,
          },
        ],
        next_cursor: null,
        retention_limit: 1000,
      },
    }));
    mocks.request.mockResolvedValue({});
    render(<NotificationCenter projectId={project} model={model} />);
    fireEvent.click(screen.getByRole("button", { name: "Open related mission" }));
    await waitFor(() =>
      expect(model.navigateResource).toHaveBeenCalledWith({
        projectId: project,
        kind: "workflow",
        resourceId: 3,
      })
    );
    await waitFor(() =>
      expect(mocks.request).toHaveBeenCalledWith(
        `/projects/${project}/notifications/9`,
        expect.objectContaining({
          method: "PATCH",
          headers: { "Content-Type": "application/json" },
        })
      )
    );
  });
  it("does not render a stale response from a different project", () => {
    mocks.read.mockImplementation(() => ({
      value: {
        project_id: "p_" + "b".repeat(32),
        unread: 1,
        items: [{ id: 1, title: "Foreign content" }],
      },
    }));
    render(<NotificationCenter projectId={project} model={model} />);
    expect(screen.queryByText("Foreign content")).toBeNull();
    expect(model.navigateResource).not.toHaveBeenCalled();
  });
});
