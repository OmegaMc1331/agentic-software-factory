import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchRunsPage } from "./api";
import App from "./App";
import { UNSAVED_CHANGES_MESSAGE } from "./unsavedChanges";

const testState = vi.hoisted(() => ({ settingsDirty: false }));

vi.mock("./api", () => ({
  fetchRun: vi.fn(),
  fetchRunsPage: vi.fn(),
  RUN_PAGE_SIZE: 25,
}));

vi.mock("./components/NetworkView", () => ({
  NetworkView: () => <div>Network view</div>,
}));

vi.mock("./components/PerformanceView", () => ({
  PerformanceView: () => <div>Performance view</div>,
}));

vi.mock("./components/RunDetailView", () => ({
  RunDetailView: () => <div>Run detail view</div>,
}));

vi.mock("./components/RunList", () => ({
  RunList: () => <div>Runs view</div>,
}));

vi.mock("./components/Settings", async () => {
  const { useUnsavedChanges } = await import("./unsavedChanges");
  return {
    SettingsView: () => {
      useUnsavedChanges(testState.settingsDirty);
      return <div>Settings view</div>;
    },
  };
});

beforeEach(() => {
  window.location.hash = "#/";
  testState.settingsDirty = false;
  vi.spyOn(window, "confirm").mockReturnValue(false);
  vi.mocked(fetchRunsPage).mockReset();
  vi.mocked(fetchRunsPage).mockResolvedValue({ items: [], nextCursor: null, hasMore: false });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("App unsaved-change navigation", () => {
  it("navigates without prompting when no editor is dirty", async () => {
    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(await screen.findByText("Settings view")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    expect(await screen.findByText("Runs view")).toBeTruthy();
    expect(window.confirm).not.toHaveBeenCalled();
    expect(window.location.hash).toBe("#/");
  });

  it("keeps the current view and URL when discard is declined", async () => {
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_CHANGES_MESSAGE);
    expect(screen.getByText("Settings view")).toBeTruthy();
    expect(window.location.hash).toBe("#/settings");
  });

  it("navigates after discard is confirmed", async () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    expect(await screen.findByText("Runs view")).toBeTruthy();
    await waitFor(() => expect(window.location.hash).toBe("#/"));
  });

  it("does not prompt when navigating to the current view", async () => {
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    expect(window.confirm).not.toHaveBeenCalled();
    expect(window.location.hash).toBe("#/settings");
  });

  it("restores the URL when an external hash change is declined", async () => {
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    act(() => {
      window.location.hash = "#/network";
    });

    await waitFor(() => expect(window.confirm).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(window.location.hash).toBe("#/settings"));
    expect(screen.getByText("Settings view")).toBeTruthy();
  });

  it("accepts an external hash change after confirmation", async () => {
    vi.mocked(window.confirm).mockReturnValue(true);
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    act(() => {
      window.location.hash = "#/network";
    });

    expect(await screen.findByText("Network view")).toBeTruthy();
    expect(window.location.hash).toBe("#/network");
  });

  it("keeps working after a cancelled external navigation", async () => {
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    act(() => {
      window.location.hash = "#/network";
    });
    await waitFor(() => expect(window.location.hash).toBe("#/settings"));

    vi.mocked(window.confirm).mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    expect(await screen.findByText("Runs view")).toBeTruthy();
    expect(window.confirm).toHaveBeenCalledTimes(2);
  });

  it("uses the shared unsaved-changes message for navigation", async () => {
    testState.settingsDirty = true;
    window.location.hash = "#/settings";
    render(<App />);

    expect(await screen.findByText("Settings view")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Runs" }));

    expect(window.confirm).toHaveBeenCalledWith(UNSAVED_CHANGES_MESSAGE);
  });
});
