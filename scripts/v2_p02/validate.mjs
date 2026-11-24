import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync, spawn } from "node:child_process";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const out = path.join(root, ".factory/v2/p02");
await fs.mkdir(out, { recursive: true });
const group = process.argv[2] ?? "frontend";
let npmCli = process.env.npm_execpath;
if (!npmCli) {
  const locations = execFileSync(
    process.platform === "win32" ? "where.exe" : "which",
    ["npm"],
    { encoding: "utf8" },
  )
    .trim()
    .split(/\r?\n/);
  for (const location of locations) {
    const candidate = path.join(
      path.dirname(location),
      "node_modules/npm/bin/npm-cli.js",
    );
    try {
      await fs.access(candidate);
      npmCli = candidate;
      break;
    } catch {
      continue;
    }
  }
}
const groups = {
  frontend: [
    ["npm", ["run", "format:check"], "apps/dashboard"],
    ["npm", ["run", "lint"], "apps/dashboard"],
    ["npm", ["run", "typecheck"], "apps/dashboard"],
    ["npm", ["test"], "apps/dashboard"],
    ["npm", ["run", "build"], "apps/dashboard"],
    ["npm", ["run", "build:v2"], "apps/dashboard"],
    ["npm", ["audit", "--json"], "apps/dashboard"],
  ],
  rust: [
    ["cargo", ["fmt", "--all", "--", "--check"], "."],
    [
      "cargo",
      [
        "clippy",
        "--workspace",
        "--all-targets",
        "-j",
        "1",
        "--",
        "-D",
        "warnings",
      ],
      ".",
    ],
    [
      "cargo",
      ["test", "--workspace", "-j", "1", "--", "--test-threads=1"],
      ".",
    ],
  ],
  compatibility: [
    ["python", ["-B", "scripts/v2_baseline.py", "inventory", "--check"], "."],
    [
      "python",
      [
        "-B",
        "-m",
        "unittest",
        "discover",
        "-s",
        "scripts",
        "-p",
        "test_v2_baseline.py",
      ],
      ".",
    ],
    ["node", ["--test", "scripts/release-checksums.test.mjs"], "."],
    ["npm", ["run", "typecheck"], "experiments/v2-ide-evaluation"],
    ["npm", ["run", "build:codemirror"], "experiments/v2-ide-evaluation"],
  ],
};
if (!groups[group]) throw new Error(`Unknown validation group: ${group}`);
const report = { group, startedAt: new Date().toISOString(), commands: [] };
for (const [command, args, cwd] of groups[group]) {
  if (command === "npm" && !npmCli)
    throw new Error("npm CLI could not be resolved");
  const index = report.commands.length;
  const log = `${group}-${String(index).padStart(2, "0")}-${command}-${args[0]}.log`;
  const stream = await fs.open(path.join(out, log), "w");
  const startedAt = new Date().toISOString();
  const start = performance.now();
  const child = spawn(
    command === "npm" ? process.execPath : command,
    command === "npm" ? [npmCli, ...args] : args,
    {
      cwd: path.join(root, cwd),
      windowsHide: true,
      stdio: ["ignore", stream.fd, stream.fd],
    },
  );
  const exitCode = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  await stream.close();
  report.commands.push({
    command,
    args,
    cwd,
    startedAt,
    durationMs: performance.now() - start,
    exitCode,
    log,
  });
  console.log(
    `${exitCode === 0 ? "PASS" : "FAIL"} ${command} ${args.join(" ")} (${Math.round((performance.now() - start) / 1000)}s)`,
  );
  await fs.writeFile(
    path.join(out, `validation-${group}.json`),
    JSON.stringify(report, null, 2),
  );
}
report.completedAt = new Date().toISOString();
await fs.writeFile(
  path.join(out, `validation-${group}.json`),
  JSON.stringify(report, null, 2),
);
process.exitCode = report.commands.some((command) => command.exitCode !== 0)
  ? 1
  : 0;
