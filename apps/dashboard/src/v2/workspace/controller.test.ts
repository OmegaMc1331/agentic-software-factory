import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspaceController } from "./controller";

beforeEach(() => {
  localStorage.clear();
  history.replaceState(null, "", "/v2.html?scenario=conversation");
});
afterEach(() => vi.restoreAllMocks());
describe("P02 local fixture state boundary", () => {
  it("loads only deterministic fixtures without contacting an API", () => {
    const request = vi.spyOn(globalThis, "fetch");
    const model = new WorkspaceController();
    expect(model.files).toHaveLength(6);
    expect(model.state.messages.every((message) => message.provenance === "fixture")).toBe(true);
    expect(request).not.toHaveBeenCalled();
  });
  it("keeps local buffer changes isolated and routes to peer views only", () => {
    const model = new WorkspaceController();
    const first = vi.fn();
    const second = vi.fn();
    model.subscribeBuffer(model.files[0].path, "view-1", first);
    const unsubscribe = model.subscribeBuffer(model.files[0].path, "view-2", second);
    model.updateBuffer(model.files[0].path, "edited fixture", "view-1");
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith("edited fixture");
    expect(model.buffer(model.files[1].path).text).not.toBe("edited fixture");
    unsubscribe();
    model.updateBuffer(model.files[0].path, "second edit", "view-1");
    expect(second).toHaveBeenCalledTimes(1);
  });
  it("does not broadcast a global React update on each typed character", () => {
    const model = new WorkspaceController();
    const notify = vi.fn();
    model.subscribe(notify);
    model.updateBuffer(model.files[0].path, "first", "editor");
    model.updateBuffer(model.files[0].path, "second", "editor");
    model.updateBuffer(model.files[0].path, "third", "editor");
    expect(notify).toHaveBeenCalledOnce();
  });
  it("records local requests with an explicit disconnected response", () => {
    const model = new WorkspaceController();
    const before = model.state.messages.length;
    model.addMessage("Explain checkpoint ownership");
    expect(model.state.messages).toHaveLength(before + 2);
    expect(model.state.messages.at(-1)?.body).toContain("No agent or runtime is connected");
    expect(model.state.messages.at(-1)?.provenance).toBe("fixture");
  });
  it("ignores empty requests", () => {
    const model = new WorkspaceController();
    const before = model.state.messages;
    model.addMessage("  ");
    expect(model.state.messages).toBe(before);
  });
  it("rejects corrupt and incompatible preview persistence with visible recovery context", () => {
    localStorage.setItem("asf.v2.p02.workspace.atlas.conversation", '{"version":999}');
    const model = new WorkspaceController();
    expect(model.state.issue).toContain("could not be restored");
    expect(model.state.messages).toHaveLength(4);
  });
  it("uses separate preview keys for fixture projects and scenarios", () => {
    const first = new WorkspaceController();
    history.replaceState(null, "", "/v2.html?scenario=long&project=fieldnotes");
    const second = new WorkspaceController();
    expect(first.storageKey).not.toBe(second.storageKey);
    expect(second.files).toHaveLength(2006);
  });
  it.each([-1, 99999, 1.5, null])("rejects invalid persisted selection offsets: %s", (anchor) => {
    const path = "src/runtime/checkpoints.rs";
    localStorage.setItem(
      "asf.v2.p02.workspace.atlas.conversation",
      JSON.stringify({
        version: 1,
        scenario: "conversation",
        project: "atlas",
        preset: "Focus",
        layout: { panels: {} },
        buffers: [[path, { text: "fixture", anchor, head: 0, scrollTop: 0 }]],
        draft: "",
        messages: [],
        review: "pending",
        selectedFile: path,
      })
    );
    const model = new WorkspaceController();
    expect(model.state.issue).toContain("could not be restored");
    expect(model.buffer(path).text).not.toBe("fixture");
  });
  it("notification dismissal changes only the selected local record", () => {
    const model = new WorkspaceController();
    const other = model.state.notifications[1];
    model.dismissNotification(model.state.notifications[0].id);
    expect(model.state.notifications).toEqual([other]);
  });
});
