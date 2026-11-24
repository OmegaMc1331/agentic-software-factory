import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { Task, TaskState } from "../types";
import { StatusBadge } from "./StatusBadge";
import { TaskGraph } from "./TaskGraph";

afterEach(cleanup);
it.each<TaskState>(["awaiting_integration", "integrating", "superseded"])(
  "preserves legacy badges and graphs for the persisted %s state",
  (state) => {
    const task: Task = {
      id: 1,
      runId: 1,
      title: "Actual task",
      objective: "Retained task",
      state,
      acceptanceCriteria: [],
      dependencies: [],
      position: 0,
      role: "worker",
      operation: "implement",
      worktreePath: null,
      agentOverride: null,
      createdAt: "2026-10-31",
      updatedAt: "2026-10-31",
    };
    render(
      <>
        <StatusBadge state={state} />
        <TaskGraph tasks={[task]} />
      </>
    );
    expect(screen.getByTitle(state.replaceAll("_", " "))).toBeTruthy();
    expect(screen.getByRole("img", { name: "task dependency graph" })).toBeTruthy();
    expect(screen.getByText("Actual task")).toBeTruthy();
  }
);
