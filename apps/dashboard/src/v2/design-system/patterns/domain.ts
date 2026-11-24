import type { Status } from "../components/Application";

export type FileReference = {
  path: string;
  language: "Rust" | "TypeScript" | "Python" | "Markdown" | "Text";
  added: number;
  removed: number;
  origin: "manual" | "agent";
  review: "pending" | "accepted";
};
export type ChatMessage = {
  id: string;
  role: "user" | "orchestrator";
  author: string;
  time: string;
  body: string;
  files?: FileReference[];
  result?: { title: string; fileCount: number; testsPassed: number; limitation?: string };
  provenance: "fixture" | "runtime";
};
export type MissionData = {
  id: string;
  title: string;
  status: Status;
  expectedResult: string;
  steps: { id: string; title: string; status: Status; detail?: string }[];
  checkpoint: { id: string; commit: string; verified: boolean };
  provenance: "fixture" | "runtime";
};
export type AgentData = {
  id: string;
  name: string;
  role: string;
  model: string;
  status: "available" | "working" | "unavailable" | "error";
  task: string;
  permissions: string[];
};
export type TestData = {
  name: string;
  status: "passed" | "failed" | "not-run";
  duration: string;
  command: string;
  provenance: "fixture" | "runtime";
};
