import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p09");
const binary =
  process.env.ASF_P08_BASELINE_BINARY ??
  path.join(root, ".factory/v2/p08-r1/run/factory-p08-r1-26dc7a7.exe");
const original = JSON.parse(
  await fs.readFile(
    path.join(root, "docs/v2/evidence/p08-r1/browser-replay.json"),
    "utf8",
  ),
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
assert.equal(hash(await fs.readFile(binary)), original.binarySha256);
const fixture = await fs.mkdtemp(
  path.join(os.tmpdir(), "asf-p09-bundle-baseline-"),
);
await fs.writeFile(
  path.join(fixture, "README.md"),
  "Disposable static-resource measurement. No agent runs or remote.\n",
);
for (const args of [
  ["init", "-q"],
  ["config", "user.name", "ASF resource measurement"],
  ["config", "user.email", "p09@example.invalid"],
  ["add", "."],
  ["commit", "-qm", "local static baseline"],
]) {
  const r = spawnSync("git", args, {
    cwd: fixture,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(r.status, 0, r.stderr);
}
const init = spawnSync(binary, ["init"], {
  cwd: fixture,
  encoding: "utf8",
  windowsHide: true,
});
assert.equal(init.status, 0, init.stderr);
const port = await new Promise((resolve) => {
  const server = net.createServer();
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolve(port));
  });
});
const log = await fs.open(path.join(output, "bundle-baseline-server.log"), "w");
const child = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    path.join(fixture, "registry.sqlite3"),
    "--allow-project-root",
    fixture,
  ],
  { cwd: fixture, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
);
await log.close();
const base = `http://127.0.0.1:${port}`;
try {
  const deadline = Date.now() + 30000;
  let ready = false;
  while (Date.now() < deadline) {
    try {
      ready = (await fetch(base + "/api/health")).ok;
    } catch {}
    if (ready) break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.ok(ready);
  const assets = [];
  for (const asset of original.assets) {
    const response = await fetch(base + "/" + asset.path);
    assert.ok(response.ok, asset.path);
    const bytes = Buffer.from(await response.arrayBuffer());
    assert.equal(hash(bytes), asset.sha256, asset.path);
    assets.push({
      path: asset.path,
      sha256: hash(bytes),
      bytes: bytes.length,
      gzipBytes: gzipSync(bytes).length,
    });
  }
  const record = {
    schema: 1,
    at: new Date().toISOString(),
    binary,
    binarySha256: original.binarySha256,
    fixture,
    modelInvocations: 0,
    qualification:
      "Actual P08-R1 embedded resource bodies, verified against historical hashes; separate disposable project with no runs or publication",
    assets,
    totalBytes: assets.reduce((n, a) => n + a.bytes, 0),
    totalGzipBytes: assets.reduce((n, a) => n + a.gzipBytes, 0),
  };
  await fs.writeFile(
    path.join(output, "bundle-baseline.json"),
    JSON.stringify(record, null, 2) + "\n",
  );
  console.log(
    JSON.stringify({
      assets: assets.length,
      bytes: record.totalBytes,
      gzipBytes: record.totalGzipBytes,
    }),
  );
} finally {
  if (child.exitCode === null) {
    child.kill();
    await new Promise((resolve) => child.once("exit", resolve));
  }
}
