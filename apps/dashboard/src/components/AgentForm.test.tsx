import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AgentForm } from "./AgentForm";
import { emptyAgentDraft, type AgentDraft } from "./agentFormState";

function draft(overrides: Partial<AgentDraft> = {}): AgentDraft {
  return { ...emptyAgentDraft(), name: "test-agent", ...overrides };
}

function environmentField(): HTMLTextAreaElement {
  return screen.getByLabelText(/Environment \(KEY=VALUE/) as HTMLTextAreaElement;
}

function submitButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Add agent" }) as HTMLButtonElement;
}

afterEach(cleanup);

describe("AgentForm environment validation", () => {
  it("saves valid environment variables, including empty values", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm
        initial={draft({ envText: "API_KEY=example\nDEBUG=true\nEMPTY=" })}
        onSave={onSave}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());

    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave).toHaveBeenCalledWith(
      "test-agent",
      expect.objectContaining({
        env: { API_KEY: "example", DEBUG: "true", EMPTY: "" },
      }),
      null
    );
  });

  it("blocks submission and shows an accessible error for malformed input", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm
        initial={draft({ envText: "INVALID_VARIABLE" })}
        onSave={onSave}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Line 1: Expected KEY=VALUE.");
    expect(onSave).not.toHaveBeenCalled();

    const field = environmentField();
    expect(field.value).toBe("INVALID_VARIABLE");
    expect(field.getAttribute("aria-invalid")).toBe("true");
    expect(field.getAttribute("aria-describedby")).toBe(alert.id);
    expect(document.activeElement).toBe(field);
    expect(submitButton().disabled).toBe(false);
  });

  it("shows the affected line number", async () => {
    render(
      <AgentForm
        initial={draft({ envText: "API_KEY=example\n\nINVALID_VARIABLE" })}
        onSave={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());

    expect((await screen.findByRole("alert")).textContent).toContain("Line 3: Expected KEY=VALUE.");
  });

  it("clears validation errors when the environment is corrected and allows resubmission", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm
        initial={draft({ envText: "INVALID_VARIABLE" })}
        onSave={onSave}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());
    expect(await screen.findByRole("alert")).toBeTruthy();

    fireEvent.change(environmentField(), { target: { value: "API_KEY=corrected" } });

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());

    fireEvent.click(submitButton());
    await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
    expect(onSave).toHaveBeenCalledWith(
      "test-agent",
      expect.objectContaining({ env: { API_KEY: "corrected" } }),
      null
    );
  });

  it("preserves values containing multiple equals signs", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm initial={draft({ envText: "TOKEN=a=b=c" })} onSave={onSave} onCancel={vi.fn()} />
    );

    fireEvent.click(submitButton());

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        "test-agent",
        expect.objectContaining({ env: { TOKEN: "a=b=c" } }),
        null
      )
    );
  });

  it("blocks duplicate variable definitions", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm
        initial={draft({ envText: "DEBUG=true\nDEBUG=false" })}
        onSave={onSave}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());

    expect((await screen.findByRole("alert")).textContent).toContain(
      "Line 2: Duplicate environment variable DEBUG"
    );
    expect(environmentField().value).toBe("DEBUG=true\nDEBUG=false");
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps existing name validation", async () => {
    const onSave = vi.fn();
    render(<AgentForm initial={draft({ name: "   " })} onSave={onSave} onCancel={vi.fn()} />);

    fireEvent.click(submitButton());

    expect(screen.getByRole("alert").textContent).toContain("Agent name is required.");
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps existing command validation", async () => {
    const onSave = vi.fn();
    render(<AgentForm initial={draft({ command: "   " })} onSave={onSave} onCancel={vi.fn()} />);

    fireEvent.click(submitButton());

    expect(screen.getByRole("alert").textContent).toContain("Command is required.");
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps existing maximum-concurrency validation", async () => {
    const onSave = vi.fn();
    const { container } = render(
      <AgentForm initial={draft({ maxConcurrency: "33" })} onSave={onSave} onCancel={vi.fn()} />
    );

    fireEvent.submit(container.querySelector("form") as HTMLFormElement);

    expect(screen.getByRole("alert").textContent).toContain(
      "Max concurrency must be an integer between 1 and 32."
    );
    expect(onSave).not.toHaveBeenCalled();
  });

  it("keeps agent presets working and preserves environment text when the type changes", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm initial={draft({ envText: "SHARED=1" })} onSave={onSave} onCancel={vi.fn()} />
    );

    fireEvent.change(screen.getByLabelText("Agent type"), { target: { value: "qwen_code" } });

    expect((screen.getByLabelText("Command") as HTMLInputElement).value).toBe("qwen");
    expect((screen.getByLabelText("Argument 1") as HTMLInputElement).value).toBe("-p");
    expect(environmentField().value).toBe("SHARED=1");

    fireEvent.click(submitButton());

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        "test-agent",
        expect.objectContaining({
          kind: "qwen_code",
          command: "qwen",
          args: ["-p"],
          env: { SHARED: "1" },
        }),
        null
      )
    );
  });

  it("keeps custom-agent configuration compatible", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    render(
      <AgentForm
        initial={draft({
          kind: "custom",
          command: "my-agent",
          args: ["run", "--flag"],
          envText: "CUSTOM=1",
          promptTransport: "argument",
          interactive: true,
          interactiveArgsText: "--interactive\n--verbose",
          maxConcurrency: "2",
          capabilities: ["coding", "review"],
        })}
        onSave={onSave}
        onCancel={vi.fn()}
      />
    );

    fireEvent.click(submitButton());

    await waitFor(() =>
      expect(onSave).toHaveBeenCalledWith(
        "test-agent",
        expect.objectContaining({
          kind: "custom",
          command: "my-agent",
          args: ["run", "--flag"],
          env: { CUSTOM: "1" },
          prompt_transport: "argument",
          interactive_args: ["--interactive", "--verbose"],
          max_concurrency: 2,
          capabilities: ["coding", "review"],
        }),
        null
      )
    );
  });

  it("keeps cancel working", () => {
    const onCancel = vi.fn();
    render(<AgentForm initial={draft()} onSave={vi.fn()} onCancel={onCancel} />);

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("keeps the loading state functional while saving", async () => {
    let resolveSave!: (value: boolean) => void;
    const onSave = vi.fn(
      () =>
        new Promise<boolean>((resolve) => {
          resolveSave = resolve;
        })
    );
    render(<AgentForm initial={draft()} onSave={onSave} onCancel={vi.fn()} />);

    fireEvent.click(submitButton());

    const saving = await screen.findByRole("button", { name: "Saving…" });
    expect((saving as HTMLButtonElement).disabled).toBe(true);

    await act(async () => {
      resolveSave(true);
    });

    await waitFor(() => expect(submitButton().disabled).toBe(false));
  });

  it("clears stale validation errors when a new initial draft is loaded", async () => {
    const onSave = vi.fn().mockResolvedValue(true);
    const onCancel = vi.fn();
    const { rerender } = render(
      <AgentForm
        initial={draft({ envText: "INVALID_VARIABLE" })}
        onSave={onSave}
        onCancel={onCancel}
      />
    );

    fireEvent.click(submitButton());
    expect(await screen.findByRole("alert")).toBeTruthy();

    rerender(
      <AgentForm
        initial={draft({ name: "other-agent", envText: "VALID=1" })}
        onSave={onSave}
        onCancel={onCancel}
      />
    );

    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("other-agent");
    expect(environmentField().value).toBe("VALID=1");
    expect(environmentField().getAttribute("aria-invalid")).toBeNull();
  });

  it("keeps server-side error handling functional", async () => {
    const onSave = vi.fn().mockRejectedValue(new Error("invalid agent configuration"));
    render(<AgentForm initial={draft()} onSave={onSave} onCancel={vi.fn()} />);

    fireEvent.click(submitButton());

    expect((await screen.findByRole("alert")).textContent).toContain("invalid agent configuration");
    expect(screen.getByRole("region", { name: "Agent configuration form" })).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
  });
});
