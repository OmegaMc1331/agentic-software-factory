import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AddNodeMenu } from "./AddNodeMenu";

afterEach(cleanup);

describe("Add Node menu", () => {
  it("rejects malformed environment lines instead of silently dropping them", () => {
    const onCreateAgent = vi.fn();
    render(
      <AddNodeMenu
        open
        config={{ agents: {}, roles: {} }}
        error={null}
        onClose={vi.fn()}
        onCreateAgent={onCreateAgent}
        onCreateRole={vi.fn()}
        onCreateVisual={vi.fn()}
      />
    );

    fireEvent.click(screen.getByRole("button", { name: /Agent/ }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "codex" } });
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "codex" } });
    fireEvent.change(screen.getByLabelText("Environment"), {
      target: { value: "VALID=1\nmissing-separator" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(screen.getByText("Environment line 2 must use KEY=VALUE.")).toBeTruthy();
    expect(onCreateAgent).not.toHaveBeenCalled();
  });
});
