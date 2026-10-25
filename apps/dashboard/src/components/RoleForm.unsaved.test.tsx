import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UNSAVED_ROLE_MESSAGE } from "../unsavedChanges";
import { RoleForm, type RoleFormInitial } from "./RoleForm";

const initial: RoleFormInitial = {
  id: "worker",
  name: "Worker",
  description: "Implements tasks",
  executionClass: "execution",
  instructions: "Do the work",
  policyPreset: "implementation",
};

beforeEach(() => {
  vi.spyOn(window, "confirm").mockReturnValue(false);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("RoleForm unsaved changes", () => {
  it("closes an unchanged edit form without prompting", async () => {
    const onCancel = vi.fn();
    const onDirtyChange = vi.fn();
    render(
      <RoleForm
        mode="edit"
        agents={["codex"]}
        initial={initial}
        submitLabel="Save role"
        onSubmit={vi.fn()}
        onCancel={onCancel}
        onDirtyChange={onDirtyChange}
      />
    );

    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(window.confirm).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("marks template application as unsaved work", async () => {
    const onCancel = vi.fn();
    const onDirtyChange = vi.fn();
    render(
      <RoleForm
        mode="create"
        agents={["codex"]}
        submitLabel="Create role"
        onSubmit={vi.fn()}
        onCancel={onCancel}
        onDirtyChange={onDirtyChange}
      />
    );

    fireEvent.change(screen.getByLabelText("Template"), { target: { value: "implementation" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.click(screen.getByRole("button", { name: "Back" }));

    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_ROLE_MESSAGE);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("keeps the role draft when discard is declined", async () => {
    const onCancel = vi.fn();
    render(
      <RoleForm
        mode="edit"
        agents={["codex"]}
        initial={initial}
        submitLabel="Save role"
        onSubmit={vi.fn()}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Changed worker" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).not.toHaveBeenCalled();
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Changed worker");
  });

  it("closes after discard is confirmed", async () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    const onCancel = vi.fn();
    render(
      <RoleForm
        mode="edit"
        agents={["codex"]}
        initial={initial}
        submitLabel="Save role"
        onSubmit={vi.fn()}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Changed worker" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("resets the baseline after a successful save", async () => {
    const onSubmit = vi.fn().mockResolvedValue(true);
    render(
      <RoleForm
        mode="edit"
        agents={["codex"]}
        initial={initial}
        submitLabel="Save role"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Saved worker" } });
    fireEvent.click(screen.getByRole("button", { name: "Save role" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("keeps the draft dirty when the save is rejected", async () => {
    const onSubmit = vi.fn().mockResolvedValue(false);
    const onCancel = vi.fn();
    render(
      <RoleForm
        mode="edit"
        agents={["codex"]}
        initial={initial}
        submitLabel="Save role"
        onSubmit={onSubmit}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Unsaved worker" } });
    fireEvent.click(screen.getByRole("button", { name: "Save role" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_ROLE_MESSAGE);
    expect(onCancel).not.toHaveBeenCalled();
  });
});
