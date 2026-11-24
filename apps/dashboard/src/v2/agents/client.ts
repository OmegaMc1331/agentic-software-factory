import { request } from "../../api";
import { projectPath } from "../projects/projectApi";
import type { Profile, ProfileInput, Handoff } from "./types";

export function saveProfile(project: string, input: ProfileInput, profile?: Profile) {
  return request<Profile>(
    projectPath(project, `agent-profiles${profile ? `/${encodeURIComponent(profile.id)}` : ""}`),
    {
      method: profile ? "PATCH" : "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(
        profile ? { expectedRevision: profile.revision, configuration: input } : input
      ),
    }
  );
}
export function archiveProfile(project: string, profile: Profile) {
  return request<Profile>(
    projectPath(project, `agent-profiles/${encodeURIComponent(profile.id)}`),
    {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ expectedRevision: profile.revision }),
    }
  );
}
export function requestHandoff(
  project: string,
  run: number,
  task: number,
  profile: Profile,
  reason: string,
  key: string
) {
  return request<Handoff>(projectPath(project, `runs/${run}/tasks/${task}/handoff`), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      idempotencyKey: key,
      toProfileId: profile.id,
      expectedProfileRevision: profile.revision,
      reason,
    }),
  });
}
