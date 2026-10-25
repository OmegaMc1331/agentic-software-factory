import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UNSAVED_RECIPE_MESSAGE } from "../unsavedChanges";
import { RecipeForm, type RecipeFormInitial } from "./RecipeForm";

const duplicate: RecipeFormInitial = {
  name: "Diagnose (copy)",
  description: "Find the cause",
  roles: ["planner", "worker"],
  routing: "project_default",
  maxParallelTasks: 2,
  policyPresets: { worker: "implementation" },
  plannerGuidance: "Keep the plan compact.",
  tools: ["docs.*"],
};

beforeEach(() => {
  vi.spyOn(window, "confirm").mockReturnValue(false);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("RecipeForm unsaved changes", () => {
  it("treats duplicated values as a clean baseline", async () => {
    const onCancel = vi.fn();
    const onDirtyChange = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
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

  it("marks an edited name as unsaved work", async () => {
    const onDirtyChange = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        onDirtyChange={onDirtyChange}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Renamed copy" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
  });

  it("becomes clean when the name is restored", async () => {
    const onDirtyChange = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={vi.fn()}
        onCancel={vi.fn()}
        onDirtyChange={onDirtyChange}
      />
    );

    const name = screen.getByLabelText("Name");
    fireEvent.change(name, { target: { value: "Renamed copy" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.change(name, { target: { value: "Diagnose (copy)" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("keeps the recipe draft when discard is declined", async () => {
    const onCancel = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={vi.fn()}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Unsaved copy" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_RECIPE_MESSAGE);
    expect(onCancel).not.toHaveBeenCalled();
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Unsaved copy");
  });

  it("closes after discard is confirmed", async () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    const onCancel = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={vi.fn()}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Unsaved copy" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("resets the baseline after a successful save", async () => {
    const onSubmit = vi.fn().mockResolvedValue(true);
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Saved copy" } });
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("keeps the draft dirty when the save is rejected", async () => {
    const onSubmit = vi.fn().mockResolvedValue(false);
    const onCancel = vi.fn();
    render(
      <RecipeForm
        mode="create"
        roles={[]}
        initial={duplicate}
        error={null}
        submitLabel="Create recipe"
        onSubmit={onSubmit}
        onCancel={onCancel}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Unsaved copy" } });
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_RECIPE_MESSAGE);
    expect(onCancel).not.toHaveBeenCalled();
  });
});
