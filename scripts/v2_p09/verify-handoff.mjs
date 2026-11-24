import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const local = path.join(root, ".factory/v2/p09");
const native = JSON.parse(
  await fs.readFile(path.join(local, "acceptance-final/results.json"), "utf8"),
);
const p = native.projects.Native;
const records = JSON.parse(
  await fs.readFile(
    path.join(root, "docs/v2/evidence/p09/agent-executions.json"),
    "utf8",
  ),
);
const previous = records.attempts.find((a) => a.status === "handed_off");
assert.ok(previous?.commit_sha);
const fixture = await fs.mkdtemp(
  path.join(os.tmpdir(), "asf-p09-handoff-verification-"),
);
const oracle =
  "from src.math_tools import clamp\nbig=10**10000\nassert clamp(big,0,big+1)==big\nassert clamp(-big,-big,big)==-big\nprint('Independent arbitrary-precision integer cases: 2')\n";
const values = [];
for (const [stage, commit, expected] of [
  ["original", p.initialHead, 0],
  ["previous-worker", previous.commit_sha, 1],
  ["integrated", p.integrationCommit, 0],
]) {
  const folder = path.join(fixture, stage);
  await fs.mkdir(path.join(folder, "src"), { recursive: true });
  const source = spawnSync("git", ["show", `${commit}:src/math_tools.py`], {
    cwd: p.root,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(source.status, 0, source.stderr);
  await fs.writeFile(path.join(folder, "src/math_tools.py"), source.stdout);
  await fs.writeFile(path.join(folder, "src/__init__.py"), "");
  const result = spawnSync("python", ["-B", "-c", oracle], {
    cwd: folder,
    encoding: "utf8",
    windowsHide: true,
    timeout: 15000,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  assert.equal(result.status, expected, result.stderr);
  if (stage === "previous-worker") assert.match(result.stderr, /OverflowError/);
  values.push({
    stage,
    commit,
    sourceSha256: createHash("sha256").update(source.stdout).digest("hex"),
    command: ["python", "-B", "-c", oracle],
    exit: result.status,
    stdout: result.stdout,
    stderr: result.stderr,
    expectedOutcome:
      stage === "previous-worker"
        ? "Recorded unapproved first Worker result exposes an independently reproduced regression"
        : "Existing valid behavior is preserved",
  });
}
const report = {
  schema: 1,
  at: new Date().toISOString(),
  kind: "Post-acceptance independent revision comparison; not a pre-mission test execution and not a harness-generated implementation patch",
  fixture,
  modelInvocations: 0,
  previousAttempt: previous.id,
  replacementAttempt: records.attempts.find(
    (a) => a.task_id === previous.task_id && a.status === "approved",
  ).id,
  results: values,
};
await fs.writeFile(
  path.join(local, "handoff-independent-verification.json"),
  JSON.stringify(report, null, 2) + "\n",
);
console.log(
  JSON.stringify(values.map((v) => ({ stage: v.stage, exit: v.exit }))),
);
