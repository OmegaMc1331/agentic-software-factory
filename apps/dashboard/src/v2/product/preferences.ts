import { request } from "../../api";
import { acknowledgeData, useData } from "../workspace/production/data";
import type { ThemePreference } from "../design-system/themes/theme";
import type { Preset } from "../workspace/production/model";

export interface ApplicationPreferences {
  schema_version: 1;
  theme: ThemePreference;
  density: "comfortable" | "compact";
  editor_font_size: number;
  editor_tab_size: number;
  editor_word_wrap: boolean;
  default_layout: Preset;
  notification_toasts: boolean;
  browser_notifications: boolean;
}
export interface PreferencesEnvelope {
  revision: number;
  preferences: ApplicationPreferences;
}
export const defaultPreferences: ApplicationPreferences = {
  schema_version: 1,
  theme: "system",
  density: "comfortable",
  editor_font_size: 13,
  editor_tab_size: 4,
  editor_word_wrap: false,
  default_layout: "Focus",
  notification_toasts: true,
  browser_notifications: false,
};
export const preferencesPath = "/application/preferences";
export const usePreferences = () => useData<PreferencesEnvelope>("", preferencesPath);
export async function savePreferences(value: PreferencesEnvelope) {
  const saved = await request<PreferencesEnvelope>(preferencesPath, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(value),
  });
  acknowledgeData("", preferencesPath, saved);
  return saved;
}
