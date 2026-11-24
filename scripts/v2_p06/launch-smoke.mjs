import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { createHash } from "node:crypto";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p06");
const binary =
  process.env.ASF_P06_BINARY ??
  path.join(
    output,
    "run",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const result = {
  schema: 1,
  binary,
  binary_sha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  started: new Date().toISOString(),
  platform: process.platform,
  checks: [],
  errors: [],
};
let child;
const run = (program, args, options = {}) => {
  const value = spawnSync(program, args, {
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
  assert.equal(value.status, 0, value.stderr);
  return value.stdout.trim();
};
const project = await fs.mkdtemp(path.join(output, "launch-smoke-"));
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p06-launch-")),
  "registry.sqlite3",
);
const env = { ...process.env };
if (process.platform === "win32") {
  const git = run("where.exe", ["git"]).split(/\r?\n/)[0];
  for (const key of Object.keys(env))
    if (key.toUpperCase() === "PATH") delete env[key];
  env.PATH = `${path.join(process.env.SystemRoot, "System32")};${path.dirname(git)}`;
  for (const name of ["node", "python", "cargo"]) {
    const checked = spawnSync("where.exe", [name], {
      env,
      encoding: "utf8",
      windowsHide: true,
    });
    assert.notEqual(
      checked.status,
      0,
      `${name} unexpectedly remained in the restricted launch PATH`,
    );
    result.checks.push(`${name} absent from child PATH`);
  }
} else
  result.checks.push(
    "Native restricted-PATH proof is Windows-specific; not qualified here",
  );
run("git", ["init", "-q", project]);
run(binary, ["init"], { cwd: project, env });
const port = await new Promise((resolve) => {
  const listener = net.createServer();
  listener.listen(0, "127.0.0.1", () => {
    const selected = listener.address().port;
    listener.close(() => resolve(selected));
  });
});
const base = `http://127.0.0.1:${port}`;
try {
  const log = await fs.open(path.join(output, "launch-smoke-server.log"), "w");
  child = spawn(
    binary,
    [
      "start",
      "--no-browser",
      "--port",
      String(port),
      "--registry",
      registry,
      "--allow-project-root",
      project,
    ],
    { cwd: project, env, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
  );
  await log.close();
  const started = performance.now();
  while (true) {
    try {
      if ((await fetch(`${base}/api/health`)).ok) break;
    } catch {}
    assert.ok(
      performance.now() - started < 30000,
      "standalone startup timed out",
    );
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  result.startup_ms = performance.now() - started;
  for (const entry of ["/", "/workspace.html"]) {
    const response = await fetch(base + entry);
    assert.equal(response.status, 200);
    assert.match(await response.text(), /assets\//);
    result.checks.push(`Embedded ${entry} served`);
  }
  const auth = await fetch(`${base}/api/auth/session`, {
    method: "POST",
    headers: { Origin: base, "Content-Type": "application/json" },
    body: "{}",
  });
  assert.equal(auth.status, 200);
  const cookie = auth.headers
    .getSetCookie()
    .map((value) => value.split(";")[0])
    .join("; ");
  const projects = await (
    await fetch(`${base}/api/projects`, { headers: { Cookie: cookie } })
  ).json();
  assert.equal(projects.length, 1);
  const state = await fetch(
    `${base}/api/projects/${projects[0].project_id}/ide/state`,
    { headers: { Cookie: cookie } },
  );
  assert.equal(state.status, 200);
  const data = await state.json();
  assert.equal(data.servers.length, 0);
  assert.equal(data.debug_sessions.length, 0);
  assert.equal(data.terminals.length, 0);
  result.checks.push(
    "Authenticated IDE state without optional runtimes or automatic tools",
  );
  result.status = "passed";
} catch (error) {
  result.errors.push(error.stack);
  result.status = "failed";
  process.exitCode = 1;
} finally {
  if (child?.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
  result.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "launch-smoke.json"),
    JSON.stringify(result, null, 2),
  );
  console.log(JSON.stringify(result, null, 2));
}
