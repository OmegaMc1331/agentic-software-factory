import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const native = JSON.parse(
  await fs.readFile(
    path.join(root, ".factory/v2/p09/acceptance-final/results.json"),
    "utf8",
  ),
);
function git(args) {
  const r = spawnSync("git", args, {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout.trim();
}
const current = git(["rev-parse", "HEAD"]);
const paths = [
  "crates/factory-agent/src/agent.rs",
  "crates/factory-core/src/factory.rs",
  "crates/factory-core/src/config.rs",
  "crates/factory-core/src/execution.rs",
  "crates/factory-core/src/scheduler.rs",
  "crates/factory-db/src/agent_profiles.rs",
  "crates/factory-db/src/scheduler.rs",
  "crates/factory-policy/src/policy.rs",
  "crates/factory-types/src/agent_profile.rs",
  "crates/factory-projects/src/manager.rs",
  "crates/factory-runtime/src/lib.rs",
];
const blobs = paths.map((file) => {
  const before = git(["rev-parse", `${native.sourceCommit}:${file}`]);
  const after = git(["rev-parse", `${current}:${file}`]);
  assert.equal(before, after, file);
  return { file, nativeBlob: before, finalBlob: after, equal: true };
});
const diff = git(["diff", native.sourceCommit, current, "--", "crates"]);
const value = {
  schema: 1,
  at: new Date().toISOString(),
  nativeSourceCommit: native.sourceCommit,
  finalSourceCommit: current,
  verifiedExecutionFiles: blobs,
  remainingCrateChanges: diff,
  qualification:
    "Listed execution, snapshot, policy, claims and process-supervision files have identical Git blobs. Remaining runtime changes distinguish already-refused handoff conditions and HTTP codes; they are separately tested. Migration assertions and additional focused tests do not alter native mission execution. Frontend changes are verified on the final embedded binary, without replaying model work.",
};
await fs.writeFile(
  path.join(root, ".factory/v2/p09/source-equivalence.json"),
  JSON.stringify(value, null, 2) + "\n",
);
console.log(
  JSON.stringify({
    verifiedFiles: blobs.length,
    native: native.sourceCommit,
    final: current,
  }),
);
