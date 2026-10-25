import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  UNSAVED_CHANGES_MESSAGE,
  confirmDiscardChanges,
  useUnsavedChanges,
  useUnsavedChangesGuard,
} from "./unsavedChanges";

function beforeUnloadCalls(spy: ReturnType<typeof vi.spyOn>) {
  return (spy.mock.calls as unknown[][]).filter((call) => call[0] === "beforeunload");
}

beforeEach(() => {
  vi.spyOn(window, "confirm").mockReturnValue(false);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("unsaved-change guards", () => {
  it("does not prompt when the editor is clean", () => {
    expect(confirmDiscardChanges(false, "Discard?")).toBe(true);
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it("keeps the editor open when discard is declined", () => {
    expect(confirmDiscardChanges(true, "Discard?")).toBe(false);
    expect(window.confirm).toHaveBeenCalledWith("Discard?");
  });

  it("allows discard when the confirmation is accepted", () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    expect(confirmDiscardChanges(true)).toBe(true);
    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_CHANGES_MESSAGE);
  });

  it("attaches a single beforeunload listener for multiple dirty editors", () => {
    const addSpy = vi.spyOn(window, "addEventListener");
    const first = renderHook(() => useUnsavedChanges(true));
    const second = renderHook(() => useUnsavedChanges(true));

    expect(beforeUnloadCalls(addSpy)).toHaveLength(1);

    first.unmount();
    second.unmount();
  });

  it("keeps the listener until every dirty editor is clean", () => {
    const removeSpy = vi.spyOn(window, "removeEventListener");
    const first = renderHook(({ dirty }) => useUnsavedChanges(dirty), {
      initialProps: { dirty: true },
    });
    const second = renderHook(({ dirty }) => useUnsavedChanges(dirty), {
      initialProps: { dirty: true },
    });

    first.rerender({ dirty: false });
    expect(beforeUnloadCalls(removeSpy)).toHaveLength(0);

    second.rerender({ dirty: false });
    expect(beforeUnloadCalls(removeSpy)).toHaveLength(1);
  });

  it("removes the listener when a dirty editor unmounts", () => {
    const removeSpy = vi.spyOn(window, "removeEventListener");
    const editor = renderHook(() => useUnsavedChanges(true));

    editor.unmount();

    expect(beforeUnloadCalls(removeSpy)).toHaveLength(1);
  });

  it("detects multiple dirty editors through the navigation guard", () => {
    const first = renderHook(() => useUnsavedChanges(true));
    const second = renderHook(() => useUnsavedChanges(true));
    const { result } = renderHook(() => useUnsavedChangesGuard());

    expect(result.current.hasUnsavedChanges()).toBe(true);
    expect(result.current.confirmDiscard()).toBe(false);
    expect(window.confirm).toHaveBeenCalledTimes(1);

    first.unmount();
    second.unmount();
    expect(result.current.hasUnsavedChanges()).toBe(false);
  });

  it("guards requestClose and closes after confirmation", () => {
    const { result } = renderHook(() => useUnsavedChanges(true));
    const close = vi.fn();

    act(() => {
      expect(result.current.requestClose(close)).toBe(false);
    });
    expect(close).not.toHaveBeenCalled();

    vi.mocked(window.confirm).mockReturnValue(true);
    act(() => {
      expect(result.current.requestClose(close)).toBe(true);
    });
    expect(close).toHaveBeenCalledTimes(1);
  });
});
