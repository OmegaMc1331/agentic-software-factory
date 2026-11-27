class HttpAssertion extends Error {
  constructor(status) {
    super("Unexpected HTTP status");
    this.status = status;
  }
}

export const KEYBOARD_CASE = "A11Y keyboard-only project, editor, settings, conversation and notification journey";
export const KEYBOARD_STAGES = Object.freeze([
  "palette_activation", "palette_focus", "palette_query", "palette_selection", "project_switch",
  "quick_open_search", "root_ready", "result_selection", "editor_focus", "editor_input",
  "disk_save", "editor_tab_mode", "settings_search", "settings_category", "settings_category_focus",
  "preference_update", "save_focus_restore", "conversation_create", "composer_focus",
  "project_return", "notification_navigation",
]);
const ROLES = new Set(["search", "editor", "result", "settings", "button", "composer", "none"]);
const ACTIONS = new Set(["switch_project", "quick_open", "settings", "conversation", "return_project", "notifications", "none"]);

export function keyboardDiagnostic(value) {
  if (!KEYBOARD_STAGES.includes(value?.checkpoint) || !ROLES.has(value.role) ||
      !ACTIONS.has(value.action) || typeof value.focusReached !== "boolean" ||
      !Number.isInteger(value.tabCount) || value.tabCount < 0 || value.tabCount > 180) {
    return undefined;
  }
  return { checkpoint: value.checkpoint, role: value.role, action: value.action,
    focusReached: value.focusReached, tabCount: value.tabCount };
}

export function requireHttpOk(response) {
  if (!response.ok()) {
    const status = response.status();
    throw new HttpAssertion(Number.isInteger(status) && status >= 100 && status <= 599 ? status : null);
  }
}

export function browserFailure(error, keyboard) {
  const result = error instanceof HttpAssertion
    ? { code: "HTTP_STATUS", expected: "2xx", actual: error.status }
    : { code: error?.code === "ERR_ASSERTION" ? "BROWSER_ASSERTION" : "BROWSER_EXECUTION" };
  const diagnostic = keyboardDiagnostic(keyboard);
  if (diagnostic) result.keyboard = diagnostic;
  return result;
}
