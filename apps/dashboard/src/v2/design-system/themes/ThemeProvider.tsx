import { useEffect, useState, type ReactNode } from "react";
import {
  applyTheme,
  readThemePreference,
  resolveTheme,
  ThemeContext,
  THEME_STORAGE_KEY,
  type ThemePreference,
} from "./theme";

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [initial] = useState(readThemePreference);
  const [preference, setValue] = useState<ThemePreference>(initial.preference);
  const [dark, setDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  const [issue, setIssue] = useState(initial.issue);
  const resolved = resolveTheme(preference, dark);
  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const change = () => setDark(query.matches);
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);
  useEffect(() => {
    applyTheme(resolved);
  }, [resolved]);
  const setPreference = (value: ThemePreference) => {
    setValue(value);
    applyTheme(resolveTheme(value, dark));
    try {
      localStorage.setItem(THEME_STORAGE_KEY, value);
      setIssue("");
    } catch {
      setIssue("Theme changed for this session. Browser storage is unavailable.");
    }
  };
  return (
    <ThemeContext.Provider value={{ preference, resolved, setPreference, issue }}>
      {children}
    </ThemeContext.Provider>
  );
}
