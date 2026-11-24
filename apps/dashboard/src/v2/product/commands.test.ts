import { afterEach, describe, it, expect, vi } from "vitest";
import {
  commandMatches,
  eligibleShortcut,
  matchesShortcut,
  shortcutConflicts,
  workspaceHasKeyboardFocus,
  type WorkspaceCommand,
} from "./commands";
afterEach(() => document.body.replaceChildren());
const command = (
  id: string,
  shortcut: string,
  extra: Partial<WorkspaceCommand> = {}
): WorkspaceCommand => ({
  id,
  title: "Open Settings",
  description: "Application theme and density",
  category: "Application",
  shortcut,
  run: vi.fn(),
  ...extra,
});
describe("production workspace command contracts", () => {
  it("keeps global commands available after focused project content is removed", () => {
    const workspace = document.createElement("main");
    const editor = document.createElement("textarea");
    workspace.append(editor);
    document.body.append(workspace);
    editor.focus();
    expect(workspaceHasKeyboardFocus(workspace)).toBe(true);
    editor.remove();
    expect(document.activeElement).toBe(document.body);
    expect(workspaceHasKeyboardFocus(workspace)).toBe(true);
  });
  it("does not claim keyboard focus from another input or an unmounted workspace", () => {
    const workspace = document.createElement("main");
    const outside = document.createElement("input");
    document.body.append(workspace, outside);
    outside.focus();
    expect(workspaceHasKeyboardFocus(workspace)).toBe(false);
    expect(workspaceHasKeyboardFocus(null)).toBe(false);
  });
  it("keeps an open dialog in control of keyboard input", () => {
    const workspace = document.createElement("main");
    const dialog = document.createElement("dialog");
    dialog.setAttribute("open", "");
    document.body.append(workspace, dialog);
    expect(workspaceHasKeyboardFocus(workspace)).toBe(false);
  });
  it("searches actual names, descriptions and shortcuts", () => {
    const c = command("settings", "Mod+,");
    expect(commandMatches(c, "theme settings")).toBe(true);
    expect(commandMatches(c, "Ctrl+,")).toBe(true);
    expect(commandMatches(c, "Cmd+,")).toBe(true);
    expect(commandMatches(c, "upload secrets")).toBe(false);
  });
  it("detects binding conflicts independent of modifier order and disables both", () => {
    const commands = [command("one", "Mod+Shift+p"), command("two", "shift+mod+P")];
    expect(shortcutConflicts(commands)).toEqual([["mod+p+shift", ["one", "two"]]]);
    expect(
      eligibleShortcut(
        new KeyboardEvent("keydown", { key: "P", ctrlKey: true, shiftKey: true }),
        commands,
        false
      )
    ).toBeUndefined();
  });
  it("honors disabled context without invoking a handler", () => {
    const c = command("save", "Mod+s", { disabled: "Open an authorized repository editor." });
    expect(
      eligibleShortcut(new KeyboardEvent("keydown", { key: "s", ctrlKey: true }), [c], false)
    ).toBeUndefined();
    expect(c.run).not.toHaveBeenCalled();
  });
  it("leaves handled CodeMirror and composing events alone", () => {
    const c = command("save", "Mod+s");
    const event = new KeyboardEvent("keydown", { key: "s", ctrlKey: true, cancelable: true });
    event.preventDefault();
    expect(eligibleShortcut(event, [c], false)).toBeUndefined();
    expect(
      eligibleShortcut(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true, isComposing: true }),
        [c],
        false
      )
    ).toBeUndefined();
  });
  it("keeps terminal keys while permitting explicitly global navigation", () => {
    const save = command("save", "Mod+s");
    const palette = command("palette", "Mod+Shift+p", { terminal: true });
    expect(
      eligibleShortcut(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
        [save, palette],
        true
      )
    ).toBeUndefined();
    expect(
      eligibleShortcut(
        new KeyboardEvent("keydown", { key: "P", ctrlKey: true, shiftKey: true }),
        [save, palette],
        true
      )?.id
    ).toBe("palette");
  });
  it("matches shifted layout digits through physical code and exact modifiers", () => {
    expect(
      matchesShortcut(
        new KeyboardEvent("keydown", { key: "!", code: "Digit1", altKey: true, shiftKey: true }),
        "Alt+Shift+1"
      )
    ).toBe(true);
    expect(
      matchesShortcut(
        new KeyboardEvent("keydown", { key: "s", ctrlKey: true, altKey: true }),
        "Mod+s"
      )
    ).toBe(false);
    expect(
      matchesShortcut(new KeyboardEvent("keydown", { key: "s", metaKey: true }), "Mod+s")
    ).toBe(true);
  });
});
