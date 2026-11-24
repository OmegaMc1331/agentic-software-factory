import "@testing-library/jest-dom/vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ThemeProvider } from "./ThemeProvider";
import { initializeTheme, readThemePreference, THEME_STORAGE_KEY, useTheme } from "./theme";

let dark = false;
const changes = new Set<() => void>();
function Probe() {
  const theme = useTheme();
  return (
    <>
      <output>{theme.resolved}</output>
      <button onClick={() => theme.setPreference("dark")}>Dark</button>
      <button onClick={() => theme.setPreference("light")}>Light</button>
      <button onClick={() => theme.setPreference("system")}>System</button>
      <span>{theme.issue}</span>
    </>
  );
}
beforeEach(() => {
  localStorage.clear();
  dark = false;
  changes.clear();
  vi.stubGlobal("matchMedia", () => ({
    get matches() {
      return dark;
    },
    addEventListener: (_: string, listener: () => void) => changes.add(listener),
    removeEventListener: (_: string, listener: () => void) => changes.delete(listener),
  }));
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("V2 themes", () => {
  it("initializes the system theme before React content", () => {
    dark = true;
    initializeTheme();
    expect(document.documentElement.dataset.asfTheme).toBe("dark");
  });
  it("persists a manual choice and restores it in a new provider", () => {
    const first = render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    );
    fireEvent.click(screen.getByText("Dark"));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(document.documentElement.dataset.asfTheme).toBe("dark");
    first.unmount();
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    );
    expect(screen.getByRole("status")).toHaveTextContent("dark");
  });
  it("follows live system changes only in system mode", () => {
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    );
    act(() => {
      dark = true;
      for (const change of changes) change();
    });
    expect(document.documentElement.dataset.asfTheme).toBe("dark");
    fireEvent.click(screen.getByText("Light"));
    act(() => {
      dark = false;
      for (const change of changes) change();
      dark = true;
      for (const change of changes) change();
    });
    expect(document.documentElement.dataset.asfTheme).toBe("light");
    fireEvent.click(screen.getByText("System"));
    expect(document.documentElement.dataset.asfTheme).toBe("dark");
  });
  it("uses system preference for an invalid stored value", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "unsupported");
    expect(readThemePreference().preference).toBe("system");
  });
  it("surfaces storage denial and still applies the selected theme", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    );
    fireEvent.click(screen.getByText("Dark"));
    expect(document.documentElement.dataset.asfTheme).toBe("dark");
    expect(screen.getByText(/Browser storage is unavailable/)).toBeInTheDocument();
  });
  it("cleans up its system-preference listener", () => {
    const view = render(
      <ThemeProvider>
        <Probe />
      </ThemeProvider>
    );
    expect(changes.size).toBe(1);
    view.unmount();
    expect(changes.size).toBe(0);
  });
});
