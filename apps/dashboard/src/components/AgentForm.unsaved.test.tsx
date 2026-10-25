import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { UNSAVED_AGENT_MESSAGE } from "../unsavedChanges";
import { AgentForm } from "./AgentForm";
import { emptyAgentDraft, type AgentDraft } from "./agentFormState";

function draft(overrides: Partial<AgentDraft> = {}): AgentDraft {
  return { ...emptyAgentDraft(), name: "test-agent", ...overrides };
}

beforeEach(() => {
  vi.spyOn(window, "confirm").mockReturnValue(false);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("AgentForm unsaved changes", () => {
  it("closes immediately when the form is clean", async () => {
    const onCancel = vi.fn();
    const onDirtyChange = vi.fn();
    render(
      <AgentForm
        initial={draft()}
        onSave={vi.fn()}
        onCancel={onCancel}
        onDirtyChange={onDirtyChange}
      />
    );

    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(window.confirm).not.toHaveBeenCalled();
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("keeps the draft when the user declines to discard it", async () => {
    const onCancel = vi.fn();
    const onDirtyChange = vi.fn();
    render(
      <AgentForm
        initial={draft()}
        onSave={vi.fn()}
        onCancel={onCancel}
        onDirtyChange={onDirtyChange}
      />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "changed" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_AGENT_MESSAGE);
    expect(onCancel).not.toHaveBeenCalled();
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("changed");
  });

  it("closes after the user confirms discarding the draft", async () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    const onCancel = vi.fn();
    render(<AgentForm initial={draft()} onSave={vi.fn()} onCancel={onCancel} />);

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "changed" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("becomes clean again when every value is restored", async () => {
    const onDirtyChange = vi.fn();
    render(
      <AgentForm
        initial={draft()}
        onSave={vi.fn()}
        onCancel={vi.fn()}
        onDirtyChange={onDirtyChange}
      />
    );

    const name = screen.getByLabelText("Name");
    fireEvent.change(name, { target: { value: "changed" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(true));
    fireEvent.change(name, { target: { value: "test-agent" } });
    await waitFor(() => expect(onDirtyChange).toHaveBeenLastCalledWith(false));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("does not reset an in-progress draft when the parent supplies new initial values", async () => {
    const { rerender } = render(
      <AgentForm initial={draft()} onSave={vi.fn()} onCancel={vi.fn()} />
    );

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "local-edit" } });
    rerender(
      <AgentForm
        initial={draft({ name: "background-refresh" })}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("local-edit");
  });

  it("establishes a new baseline after a successful save", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(<AgentForm initial={draft()} onSave={onSave} onCancel={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "saved-agent" } });
    fireEvent.click(screen.getByRole("button", { name: "Add agent" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("blocks duplicate submissions while a save is pending", async () => {
    let resolveSave!: (value: boolean) => void;
    const onSave = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveSave = resolve;
        })
    );
    const { container } = render(
      <AgentForm initial={draft()} onSave={onSave} onCancel={vi.fn()} />
    );
    const form = container.querySelector("form") as HTMLFormElement;

    fireEvent.submit(form);
    fireEvent.submit(form);

    expect(onSave).toHaveBeenCalledTimes(1);
    await act(async () => resolveSave(true));
  });
});
