import fs from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output =
  process.env.ASF_P10_REGRESSIONS ??
  path.join(root, ".factory/v2/p10/regressions");
const binary = process.env.ASF_P10_BINARY;
if (!binary) throw new Error("ASF_P10_BINARY must select the P10 executable");
await fs.mkdir(output, { recursive: true });
try {
  await fs.mkdir(path.join(output, "attempts"), { recursive: true });
  await fs.copyFile(
    path.join(output, "results.json"),
    path.join(output, "attempts", `regressions-${Date.now()}.json`),
  );
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const result = {
  schema: 1,
  started: new Date().toISOString(),
  binary,
  binarySha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  kind: "Existing production browser regressions with deterministic agent fixtures; not native model qualification",
  cases: [],
};
const env = {
  ...process.env,
  ASF_P07_REAL_MODE: "controlled",
  ASF_IDE_TEST_TOOLS:
    process.env.ASF_IDE_TEST_TOOLS ??
    path.join(root, ".factory/v2/p06/tools/profiles.json"),
};
for (const phase of ["p03", "p04", "p05", "p06", "p07"]) {
  env[`ASF_${phase.toUpperCase()}_BINARY`] = binary;
  env[`ASF_${phase.toUpperCase()}_OUTPUT_DIR`] = path.join(output, phase);
}
for (const [phase, script, filename] of [
  ["p03", "scripts/v2_p03/browser.mjs", "browser-results.json"],
  ["p04", "scripts/v2_p04/browser.mjs", "browser-results.json"],
  ["p05", "scripts/v2_p05/browser.mjs", "editor-browser-results.json"],
  ["p06", "scripts/v2_p06/browser.mjs", "browser-results.json"],
  ["p07", "scripts/v2_p07/browser.mjs", "browser-results.json"],
  [
    "p07",
    "scripts/v2_p07/recovery-browser.mjs",
    "recovery-browser-results.json",
  ],
]) {
  const destination = path.join(output, phase);
  await fs.mkdir(destination, { recursive: true });
  const log = await fs.open(
    path.join(destination, `${path.basename(script, ".mjs")}-runner.log`),
    "w",
  );
  const started = performance.now();
  const child = spawn(process.execPath, [script], {
    cwd: root,
    env,
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  });
  await log.close();
  const exit = await new Promise((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
  });
  const evidence = JSON.parse(
    await fs.readFile(path.join(destination, filename), "utf8"),
  );
  result.cases.push({
    phase,
    script,
    exit,
    durationMs: performance.now() - started,
    count: evidence.tests.length,
    tests: evidence.tests,
  });
  await fs.writeFile(
    path.join(output, "results.json"),
    JSON.stringify(result, null, 2) + "\n",
  );
  console.log(`${phase}: ${evidence.tests.length} scenarios, exit ${exit}`);
  if (exit !== 0 || evidence.tests.some((test) => test.status !== "passed"))
    throw new Error(`Regression failed: ${script}`);
}
result.finished = new Date().toISOString();
await fs.writeFile(
  path.join(output, "results.json"),
  JSON.stringify(result, null, 2) + "\n",
);
