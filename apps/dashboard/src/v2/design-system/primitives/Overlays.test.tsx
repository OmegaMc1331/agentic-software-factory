import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { Button } from "./Controls";
import { ContextMenu, ConfirmationDialog, Dialog, Dropdown, Popover, Tooltip } from "./Overlays";

beforeEach(() => {
  Object.defineProperty(HTMLDialogElement.prototype, "showModal", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.open = true;
    },
  });
  Object.defineProperty(HTMLDialogElement.prototype, "close", {
    configurable: true,
    value: function (this: HTMLDialogElement) {
      this.open = false;
    },
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("V2 overlays", () => {
  it("menus focus enabled entries, navigate and return focus", async () => {
    const action = vi.fn();
    render(
      <Dropdown
        label="Actions"
        entries={[
          { id: "one", label: "First", action },
          { id: "disabled", label: "Unavailable", disabled: true, action },
          { id: "last", label: "Last", action },
        ]}
      />
    );
    const trigger = screen.getByRole("button", { name: "Actions" });
    trigger.focus();
    fireEvent.click(trigger);
    expect(screen.getByRole("menuitem", { name: "First" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(screen.getByRole("menuitem", { name: "Last" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(trigger).toHaveFocus();
    expect(action).not.toHaveBeenCalled();
  });
  it("menu actions activate once and close the popup", () => {
    const action = vi.fn();
    render(<Dropdown label="Menu" entries={[{ id: "one", label: "Apply", action }]} />);
    fireEvent.click(screen.getByRole("button"));
    fireEvent.click(screen.getByRole("menuitem"));
    expect(action).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });
  it("context menus support pointer and Shift F10 invocation", () => {
    render(
      <ContextMenu
        label="File actions"
        entries={[{ id: "open", label: "Open", action: () => undefined }]}
      >
        <Button>File</Button>
      </ContextMenu>
    );
    const source = screen.getByRole("button");
    source.focus();
    fireEvent.keyDown(source, { key: "F10", shiftKey: true });
    expect(screen.getByRole("menu", { name: "File actions" })).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("menuitem"), { key: "Escape" });
    fireEvent.contextMenu(source, { clientX: 100, clientY: 80 });
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });
  it("outside pointer closes menus without activating an entry", () => {
    const action = vi.fn();
    render(<Dropdown label="Actions" entries={[{ id: "one", label: "First", action }]} />);
    fireEvent.click(screen.getByRole("button"));
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(action).not.toHaveBeenCalled();
  });
  it("dialogs expose names/descriptions and cancel without confirming", () => {
    function Example() {
      const [open, set] = useState(true);
      return (
        <Dialog
          open={open}
          onClose={() => set(false)}
          title="Preferences"
          description="Change preview appearance"
        >
          <Button>Apply</Button>
        </Dialog>
      );
    }
    render(<Example />);
    const dialog = screen.getByRole("dialog", { name: "Preferences" });
    expect(dialog).toHaveAccessibleDescription("Change preview appearance");
    fireEvent(dialog, new Event("cancel", { bubbles: false, cancelable: true }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("confirmation distinguishes cancellation from explicit confirmation", () => {
    const accept = vi.fn();
    const close = vi.fn();
    render(
      <ConfirmationDialog
        open
        onClose={close}
        onConfirm={accept}
        title="Remove fixture?"
        description="No project data changes"
        confirmLabel="Remove fixture"
        danger
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(close).toHaveBeenCalledOnce();
    expect(accept).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Remove fixture" }));
    expect(accept).toHaveBeenCalledOnce();
  });
  it("popovers have accessible labels and dismiss on Escape", () => {
    render(
      <Popover label="Context" trigger="Show context">
        <p>Architecture decisions</p>
      </Popover>
    );
    fireEvent.click(screen.getByRole("button"));
    const dialog = screen.getByRole("dialog", { name: "Context" });
    expect(dialog).toHaveFocus();
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("tooltips appear after intent, describe the control and dismiss on Escape", async () => {
    render(
      <Tooltip content="Opens context">
        <Button>Context</Button>
      </Tooltip>
    );
    fireEvent.focus(screen.getByRole("button"));
    await screen.findByRole("tooltip");
    expect(screen.getByRole("button")).toHaveAccessibleDescription("Opens context");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
  });
});
