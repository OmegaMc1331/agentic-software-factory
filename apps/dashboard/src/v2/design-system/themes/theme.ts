import { createContext, useContext } from "react";

export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";
export const THEME_STORAGE_KEY = "asf.v2.p02.theme";

export function readThemePreference(): { preference: ThemePreference; issue: string } {
  try {
    const value = localStorage.getItem(THEME_STORAGE_KEY);
    return { preference: value === "dark" || value === "light" ? value : "system", issue: "" };
  } catch {
    return {
      preference: "system",
      issue: "Theme preference cannot be saved in this browser session.",
    };
  }
}

export function resolveTheme(
  preference: ThemePreference,
  dark = window.matchMedia("(prefers-color-scheme: dark)").matches
): ResolvedTheme {
  return preference === "system" ? (dark ? "dark" : "light") : preference;
}

export function initializeTheme() {
  document.documentElement.dataset.asfTheme = resolveTheme(readThemePreference().preference);
}

let themeTimer: ReturnType<typeof setTimeout> | undefined;
export function applyTheme(theme: ResolvedTheme) {
  const root = document.documentElement;
  if (root.dataset.asfTheme === theme) return;
  root.dataset.themeChanging = "true";
  root.dataset.asfTheme = theme;
  clearTimeout(themeTimer);
  themeTimer = setTimeout(() => {
    delete root.dataset.themeChanging;
  }, 230);
}

export const ThemeContext = createContext<{
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference(value: ThemePreference): void;
  issue: string;
} | null>(null);

export function useTheme() {
  const context = useContext(ThemeContext);
  if (!context) throw new Error("ThemeProvider is required");
  return context;
}
