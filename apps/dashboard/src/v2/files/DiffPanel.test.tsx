import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Text } from "@codemirror/state";
import { DiffPanel } from "./DiffPanel";
import type { EditorDocument } from "./documents";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import { createSession, createView } from "../workspace/production/model";

afterEach(cleanup);
describe("restored file comparisons", () => {
  it.each(["metadata", "preview"] as const)(
    "refuses a restored base diff for %s-only content",
    (mode) => {
      const projectId = `p_${"a".repeat(32)}`;
      const file = {
        project_id: projectId,
        root_id: `r_${"b".repeat(32)}`,
        path: "unsupported.bin",
      };
      const document = {
        id: `d_${"c".repeat(32)}`,
        file,
        status: "ready",
        text: Text.empty,
        baseText: Text.empty,
        baseRevision: "v_observed_bytes",
        generation: 0,
        disk: {
          file,
          revision: "v_observed_bytes",
          content: mode === "metadata" ? null : "partial preview",
          mode,
          read_only: true,
        },
      } as EditorDocument;
      const model = {
        documents: {
          get: () => document,
          subscribe: () => () => {},
          open: async () => document.id,
        },
      } as unknown as WorkspaceModel;
      const view = createView(createSession(projectId), "diff", {
        file,
        documentId: document.id,
        comparison: "base",
      });
      const result = render(<DiffPanel view={view} model={model} dock={{} as DockSession} />);
      expect(screen.getByText("Text comparison unavailable")).toBeTruthy();
      expect(result.container.querySelector(".cm-mergeView")).toBeNull();
    }
  );
});
