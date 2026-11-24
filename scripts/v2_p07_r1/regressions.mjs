import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p07-r1/regressions");
const binary =
  process.env.ASF_P07_R1_BINARY ??
  path.join(root, ".factory/v2/p07-r1/run/factory-r1.exe");
const hash = createHash("sha256")
  .update(await fs.readFile(binary))
  .digest("hex");
await fs.mkdir(output, { recursive: true });
const results = {
  started: new Date().toISOString(),
  binarySha256: hash,
  cases: [],
};
const cases = [
  [
    "p07",
    "scripts/v2_p07/browser.mjs",
    path.join(output, "p07"),
    "browser-results.json",
  ],
  [
    "recovery",
    "scripts/v2_p07/recovery-browser.mjs",
    path.join(output, "p07"),
    "recovery-browser-results.json",
  ],
  ...["p03", "p04", "p05", "p06"].map((phase) => [
    phase,
    `scripts/v2_${phase}/browser.mjs`,
    path.join(root, `.factory/v2/${phase}`),
    phase === "p05" ? "editor-browser-results.json" : "browser-results.json",
  ]),
];
for (const [phase, script, source, filename] of cases) {
  if (
    ["p03", "p04", "p05"].includes(phase) &&
    process.argv.includes("--existing-p03-p05")
  ) {
    const prior = JSON.parse(
      await fs.readFile(path.join(source, filename), "utf8"),
    );
    if (
      path.resolve(prior.binary) !== path.resolve(binary) ||
      prior.tests.some((test) => test.status !== "passed")
    )
      throw new Error(
        "Only passing regressions on this binary copy may be reused",
      );
    const destination = path.join(output, phase);
    await fs.mkdir(destination, { recursive: true });
    await fs.copyFile(
      path.join(source, filename),
      path.join(destination, filename),
    );
    results.cases.push({
      phase,
      reusedIdenticalBinaryRun: true,
      count: prior.tests.length,
    });
    continue;
  }
  if (
    (phase === "p07" && process.argv.includes("--existing-p07")) ||
    (phase === "recovery" && process.argv.includes("--existing-recovery"))
  ) {
    const prior = JSON.parse(
      await fs.readFile(path.join(source, filename), "utf8"),
    );
    if (
      prior.binarySha256 !== hash ||
      prior.tests.some((test) => test.status !== "passed")
    )
      throw new Error(
        "The existing P07 run must be passing on the identical binary",
      );
    results.cases.push({
      phase,
      reusedIdenticalBinaryRun: true,
      count: prior.tests.length,
    });
    continue;
  }
  const destination = path.join(output, phase);
  await fs.mkdir(destination, { recursive: true });
  const log = await fs.open(path.join(destination, "runner.log"), "w");
  const start = performance.now();
  const processValue = spawn(process.execPath, [script], {
    cwd: root,
    windowsHide: true,
    env: {
      ...process.env,
      ASF_P07_OUTPUT_DIR: path.join(output, "p07"),
      ASF_P07_REAL_MODE: "controlled",
      ASF_P07_BINARY: binary,
      ASF_P03_BINARY: binary,
      ASF_P04_BINARY: binary,
      ASF_P05_BINARY: binary,
      ASF_P06_BINARY: binary,
    },
    stdio: ["ignore", log.fd, log.fd],
  });
  await log.close();
  const exit = await new Promise((resolve, reject) => {
    processValue.once("exit", resolve);
    processValue.once("error", reject);
  });
  const record = {
    phase,
    command: [process.execPath, script],
    cwd: root,
    exit,
    ms: performance.now() - start,
  };
  const artifact = path.join(source, filename);
  const value = JSON.parse(await fs.readFile(artifact, "utf8"));
  record.count = value.tests.length;
  record.passed = value.tests.filter((test) => test.status === "passed").length;
  if (artifact !== path.join(destination, filename))
    await fs.copyFile(artifact, path.join(destination, filename));
  results.cases.push(record);
  await fs.writeFile(
    path.join(output, "results.json"),
    JSON.stringify(results, null, 2),
  );
  console.log(
    `${exit === 0 ? "PASS" : "FAIL"} ${phase}: ${record.passed}/${record.count}`,
  );
  if (exit !== 0) {
    process.exitCode = 1;
    break;
  }
}
results.finished = new Date().toISOString();
await fs.writeFile(
  path.join(output, "results.json"),
  JSON.stringify(results, null, 2),
);
