import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
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
} from "./Controls";
import {
  ConfirmationPrompt,
  FileTreeItem,
  PanelToolbar,
  ResizableDivider,
  WorkspaceTab,
} from "../components/Application";

afterEach(cleanup);

it("checkbox validation describes the error without changing its accessible name", () => {
  render(<Checkbox label="Include context" error="Choose verified entries only" />);
  const checkbox = screen.getByRole("checkbox", { name: "Include context" });
  expect(checkbox).toHaveAccessibleDescription("Choose verified entries only");
  expect(checkbox).toHaveAttribute("aria-invalid", "true");
});

describe("V2 controls", () => {
  it.each(["primary", "neutral", "danger", "quiet"] as const)(
    "%s buttons keep native semantics and activate once",
    (tone) => {
      const action = vi.fn();
      render(
        <Button tone={tone} onClick={action}>
          Apply
        </Button>
      );
      const button = screen.getByRole("button", { name: "Apply" });
      expect(button).toHaveAttribute("type", "button");
      fireEvent.click(button);
      expect(action).toHaveBeenCalledOnce();
    }
  );
  it.each([{ disabled: true }, { loading: true }])(
    "disabled/busy buttons refuse activation: %o",
    (props) => {
      const action = vi.fn();
      render(
        <Button {...props} onClick={action}>
          Save
        </Button>
      );
      expect(screen.getByRole("button")).toBeDisabled();
      fireEvent.click(screen.getByRole("button"));
      expect(action).not.toHaveBeenCalled();
      if ("loading" in props)
        expect(screen.getByRole("button")).toHaveAttribute("aria-busy", "true");
    }
  );
  it("icon buttons require and expose an accessible action name", () => {
    render(<IconButton label="Split editor" icon="split" />);
    expect(screen.getByRole("button", { name: "Split editor" })).toBeInTheDocument();
  });
  it("inputs associate labels, hints and validation errors", () => {
    const { rerender } = render(<Input label="Session" hint="Use a recognizable name" />);
    expect(screen.getByRole("textbox", { name: "Session" })).toHaveAccessibleDescription(
      "Use a recognizable name"
    );
    rerender(<Input label="Session" error="A name is required" />);
    expect(screen.getByRole("textbox")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("textbox")).toHaveAccessibleDescription("A name is required");
  });
  it("textareas and selects keep native labels and disabled state", () => {
    render(
      <>
        <Textarea label="Goal" disabled />
        <Select label="Scope" disabled>
          <option>Project</option>
        </Select>
      </>
    );
    expect(screen.getByRole("textbox", { name: "Goal" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Scope" })).toBeDisabled();
  });
  it("checkboxes and radios use native checked semantics", () => {
    render(
      <>
        <Checkbox label="Include context" defaultChecked />
        <RadioControl label="Project" name="scope" defaultChecked />
        <RadioControl label="Worktree" name="scope" />
      </>
    );
    expect(screen.getByRole("checkbox")).toBeChecked();
    fireEvent.click(screen.getByRole("checkbox"));
    expect(screen.getByRole("checkbox")).not.toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: "Worktree" }));
    expect(screen.getByRole("radio", { name: "Worktree" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Project" })).not.toBeChecked();
  });
  it("toggles expose a controlled checked state and honor disabling", () => {
    const action = vi.fn();
    const { rerender } = render(<Toggle label="Compact" checked onChange={action} />);
    fireEvent.click(screen.getByRole("switch"));
    expect(action).toHaveBeenCalledWith(false);
    rerender(<Toggle label="Compact" checked={false} disabled onChange={action} />);
    fireEvent.click(screen.getByRole("switch"));
    expect(action).toHaveBeenCalledTimes(1);
  });
  it("search clears on Escape and with its named clear button", () => {
    function Search() {
      const [value, set] = useState("checkpoint");
      return <SearchField label="Find fixture" value={value} onChange={set} />;
    }
    render(<Search />);
    const input = screen.getByRole("searchbox");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(input).toHaveValue("");
    fireEvent.change(input, { target: { value: "owner" } });
    fireEvent.click(screen.getByRole("button", { name: "Clear find fixture" }));
    expect(input).toHaveValue("");
  });
  it("tabs skip disabled items, focus the selected tab and wrap", () => {
    function Example() {
      const [value, set] = useState("one");
      return (
        <Tabs
          label="Modes"
          value={value}
          onChange={set}
          items={[
            { id: "one", label: "One" },
            { id: "disabled", label: "Disabled", disabled: true },
            { id: "two", label: "Two" },
          ]}
        />
      );
    }
    render(<Example />);
    screen.getByRole("tab", { name: "One" }).focus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "Two" })).toHaveFocus();
    expect(screen.getByRole("tab", { name: "Two" })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(document.activeElement!, { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "One" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "End" });
    expect(screen.getByRole("tab", { name: "Two" })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "Home" });
    expect(screen.getByRole("tab", { name: "One" })).toHaveFocus();
  });
  it("toolbars provide one tab stop and arrow navigation", () => {
    render(
      <PanelToolbar label="Actions">
        <Button>First</Button>
        <Button disabled>Unavailable</Button>
        <Button>Last</Button>
      </PanelToolbar>
    );
    const toolbar = screen.getByRole("toolbar", { name: "Actions" });
    const first = within(toolbar).getByRole("button", { name: "First" });
    const last = within(toolbar).getByRole("button", { name: "Last" });
    expect(first.tabIndex).toBe(0);
    expect(last.tabIndex).toBe(-1);
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(last).toHaveFocus();
    fireEvent.keyDown(last, { key: "ArrowRight" });
    expect(first).toHaveFocus();
  });
  it("workspace tabs communicate unsaved state and movement requests", () => {
    const move = vi.fn();
    const close = vi.fn();
    render(
      <div role="tablist">
        <WorkspaceTab
          id="file"
          label="main.rs"
          selected
          dirty
          onSelect={() => undefined}
          onClose={close}
          onMove={move}
        />
      </div>
    );
    expect(screen.getByRole("tab")).toHaveAttribute("aria-selected", "true");
    expect(screen.getByLabelText("Unsaved changes")).toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole("tab"), { key: "ArrowRight", altKey: true });
    expect(move).toHaveBeenCalledWith("right");
    fireEvent.keyDown(screen.getByRole("tab", { name: /main.rs/ }), { key: "Delete" });
    expect(close).toHaveBeenCalledOnce();
  });
  it("tree items expose level, selection, expansion and complete path", () => {
    render(
      <div role="tree">
        <FileTreeItem
          path="src/nested"
          label="nested"
          depth={1}
          directory
          expanded
          selected
          onSelect={() => undefined}
          position={2}
          total={3}
        />
      </div>
    );
    const item = screen.getByRole("treeitem");
    expect(item).toHaveAttribute("aria-level", "2");
    expect(item).toHaveAttribute("aria-selected", "true");
    expect(item).toHaveAttribute("aria-expanded", "true");
    expect(item).toHaveAccessibleName("src/nested");
  });
  it("resizers clamp keyboard changes and expose range semantics", () => {
    const change = vi.fn();
    render(
      <ResizableDivider
        label="Inspector width"
        controls="inspector"
        value={200}
        min={160}
        max={300}
        onChange={change}
      />
    );
    const separator = screen.getByRole("separator");
    fireEvent.keyDown(separator, { key: "ArrowRight", shiftKey: true });
    expect(change).toHaveBeenLastCalledWith(232);
    fireEvent.keyDown(separator, { key: "Home" });
    expect(change).toHaveBeenLastCalledWith(160);
    fireEvent.keyDown(separator, { key: "End" });
    expect(change).toHaveBeenLastCalledWith(300);
  });
  it("inline confirmation has distinct accept and keep-current actions", () => {
    const accept = vi.fn();
    const keep = vi.fn();
    render(
      <ConfirmationPrompt
        title="Checkpoint"
        description="Preserve changes"
        confirmLabel="Use checkpoint"
        onConfirm={accept}
        onCancel={keep}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Use checkpoint" }));
    fireEvent.click(screen.getByRole("button", { name: "Keep current state" }));
    expect(accept).toHaveBeenCalledOnce();
    expect(keep).toHaveBeenCalledOnce();
  });
});
