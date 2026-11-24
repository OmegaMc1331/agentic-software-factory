import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const directory = path.join(root, "docs/v2/evidence/p09");
const manifest = JSON.parse(
  await fs.readFile(path.join(directory, "manifest.json"), "utf8"),
);
assert.deepEqual(manifest.pendingGates, []);
let total = 0;
const credentialPatternPaths = [];
for (const file of manifest.files) {
  assert.ok(
    !file.path.split("/").includes("..") && !path.isAbsolute(file.path),
  );
  assert.ok(!/\.(exe|sqlite3|db|stderr)$/.test(file.path));
  const bytes = await fs.readFile(path.join(directory, file.path));
  assert.equal(
    createHash("sha256").update(bytes).digest("hex"),
    file.sha256,
    file.path,
  );
  assert.equal(bytes.length, file.bytes, file.path);
  total += bytes.length;
  if (
    /\.(json|md|txt)$/.test(file.path) &&
    /(sk-proj-[A-Za-z0-9_-]{12,}|Bearer [A-Za-z0-9_-]{24,}|"csrfToken"\s*:\s*"[^"]+")/.test(
      bytes.toString("utf8"),
    )
  )
    credentialPatternPaths.push(file.path);
  if (process.argv.includes("--git")) {
    const blob = spawnSync(
      "git",
      ["show", `HEAD:docs/v2/evidence/p09/${file.path}`],
      { cwd: root, windowsHide: true, maxBuffer: 16 * 1024 * 1024 },
    );
    assert.equal(blob.status, 0, String(blob.stderr));
    assert.equal(
      createHash("sha256").update(blob.stdout).digest("hex"),
      file.sha256,
      "Git blob: " + file.path,
    );
  }
}
assert.deepEqual(credentialPatternPaths, []);
const executions = JSON.parse(
  await fs.readFile(path.join(directory, "agent-executions.json"), "utf8"),
);
assert.ok(
  executions.sessions.every(
    (s) => !Object.hasOwn(s, "stderr") && Array.isArray(s.native_header),
  ),
);
console.log(
  JSON.stringify({
    hashedFiles: manifest.files.length,
    totalBytes: total,
    credentialPatternPaths,
    gitBlobsChecked: process.argv.includes("--git"),
    qualification:
      "Selected high-risk credential pattern check; not a universal secret-detection guarantee. Native raw stderr is excluded.",
  }),
);
