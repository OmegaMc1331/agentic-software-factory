import type { DockviewApi } from "dockview";

export type Preset = "Focus" | "Code" | "Agents" | "Review";
export const primaryPath = "src/runtime/checkpoints.rs";
export type ViewName =
  "conversation" | "explorer" | "mission" | "agents" | "output" | "review" | "diff" | "editor";
export const viewTitles: Record<ViewName, string> = {
  conversation: "Orchestrator",
  explorer: "Explorer",
  mission: "Progress",
  agents: "Agents",
  output: "Output",
  review: "Review",
  diff: "Changes · checkpoints.rs",
  editor: "checkpoints.rs",
};

export function addView(
  api: DockviewApi,
  name: ViewName,
  options: {
    reference?: string;
    direction?: "left" | "right" | "above" | "below" | "within";
    width?: number;
    height?: number;
  } = {}
) {
  return api.addPanel({
    id: name,
    component: name,
    title: viewTitles[name],
    params: name === "editor" || name === "diff" ? { path: primaryPath } : {},
    position: options.reference
      ? { referencePanel: options.reference, direction: options.direction ?? "within" }
      : undefined,
    initialWidth: options.direction === "within" ? undefined : options.width,
    initialHeight: options.direction === "within" ? undefined : options.height,
  });
}

export function applyPreset(api: DockviewApi, preset: Preset, narrow: boolean) {
  api.clear();
  if (preset === "Focus") {
    addView(api, "conversation");
    addView(api, "explorer", {
      reference: "conversation",
      direction: "left",
      width: narrow ? 200 : 224,
    });
    if (!narrow)
      addView(api, "mission", { reference: "conversation", direction: "right", width: 286 });
    api.getPanel("conversation")?.api.setActive();
  } else if (preset === "Code") {
    addView(api, "editor");
    addView(api, "explorer", { reference: "editor", direction: "left", width: narrow ? 200 : 224 });
    addView(api, "conversation", {
      reference: "editor",
      direction: narrow ? "within" : "right",
      width: 340,
    });
    addView(api, "output", { reference: "editor", direction: "below", height: 170 });
    api.getPanel("editor")?.api.setActive();
  } else if (preset === "Agents") {
    addView(api, "conversation");
    addView(api, "agents", {
      reference: "conversation",
      direction: "right",
      width: narrow ? 310 : 370,
    });
    addView(api, "mission", {
      reference: "agents",
      direction: narrow ? "within" : "below",
      height: 275,
    });
    if (narrow) api.getPanel("agents")?.api.setActive();
    api.getPanel("conversation")?.api.setActive();
  } else {
    addView(api, "diff");
    addView(api, "explorer", { reference: "diff", direction: "left", width: narrow ? 200 : 224 });
    addView(api, "review", {
      reference: "diff",
      direction: narrow ? "within" : "right",
      width: 300,
    });
    addView(api, "output", { reference: "diff", direction: "below", height: 165 });
    addView(api, "conversation", { reference: "diff", direction: "within" });
    api.getPanel("diff")?.api.setActive();
  }
  for (const panel of api.panels)
    panel.api.setConstraints({
      minimumWidth: panel.id === "explorer" ? 160 : 220,
      minimumHeight: panel.id === "output" ? 100 : 120,
    });
}
