import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p05/logs");
fs.mkdirSync(output, { recursive: true });
const mode = process.argv[2] ?? "targeted";
const commands = {
  targeted: [
    [
      "cargo",
      ["test", "-p", "factory-files", "-j", "1", "--", "--test-threads=1"],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-api",
        "--test",
        "files",
        "--test",
        "multi_project",
        "--test",
        "workspaces",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  rust: [
    ["cargo", ["fmt", "--all", "--", "--check"]],
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
    ],
    ["cargo", ["test", "--workspace", "-j", "1", "--", "--test-threads=1"]],
  ],
  parallel: [["cargo", ["test", "--workspace", "-j", "1"]]],
  frontend: [
    ["npm", ["run", "format:check"]],
    ["npm", ["run", "lint"]],
    ["npm", ["run", "typecheck"]],
    ["npm", ["test"]],
    ["npm", ["run", "build"]],
  ],
  "frontend-default": [["npm", ["test"]]],
  "frontend-1": [["npm", ["test", "--", "--maxWorkers=1"]]],
  "frontend-2": [["npm", ["test", "--", "--maxWorkers=2"]]],
  embedded: [
    [
      "cargo",
      [
        "build",
        "-p",
        "factory-cli",
        "--features",
        "embedded-dashboard",
        "-j",
        "1",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-api",
        "--lib",
        "--features",
        "embedded-dashboard",
        "dashboard::embedded",
        "-j",
        "1",
      ],
    ],
  ],
};
if (!commands[mode]) throw new Error(`Unknown validation mode ${mode}`);
const results = {
  schema: 1,
  mode,
  started: new Date().toISOString(),
  platform: process.platform,
  cpus: os.cpus().length,
  totalMemory: os.totalmem(),
  commands: [],
};
const memory = (pid) => {
  if (process.platform !== "win32") return undefined;
  const script = `$items=Get-CimInstance Win32_Process; $owned=[System.Collections.Generic.HashSet[int]]::new(); [void]$owned.Add(${pid}); for($i=0;$i -lt 8;$i++){foreach($item in $items){if($owned.Contains([int]$item.ParentProcessId)){[void]$owned.Add([int]$item.ProcessId)}}}; $processes=@(foreach($idValue in $owned){Get-Process -Id $idValue -ErrorAction SilentlyContinue}); $system=Get-CimInstance Win32_OperatingSystem; @{rss=($processes|Measure-Object WorkingSet64 -Sum).Sum; privateBytes=($processes|Measure-Object PrivateMemorySize64 -Sum).Sum; count=$processes.Count; freePhysical=[int64]$system.FreePhysicalMemory*1024; freeCommit=[int64]$system.FreeVirtualMemory*1024}|ConvertTo-Json -Compress`;
  const value = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { encoding: "utf8", windowsHide: true, timeout: 8000 },
  );
  try {
    return JSON.parse(value.stdout);
  } catch {
    return undefined;
  }
};
for (const [program, args] of commands[mode]) {
  const name = `${mode}-${results.commands.length + 1}`,
    file = fs.openSync(path.join(output, `${name}.log`), "w");
  const cwd = program === "npm" ? path.join(root, "apps/dashboard") : root;
  const start = performance.now();
  const executable =
    program === "npm" && process.platform === "win32" ? "cmd.exe" : program;
  const argumentsValue =
    executable === "cmd.exe"
      ? ["/d", "/s", "/c", `npm ${args.join(" ")}`]
      : args;
  const child = spawn(executable, argumentsValue, {
    cwd,
    env: {
      ...process.env,
      CARGO_TARGET_DIR:
        process.env.CARGO_TARGET_DIR ?? path.join(root, "target"),
      CARGO_INCREMENTAL: "0",
      CARGO_PROFILE_DEV_DEBUG: "0",
      CARGO_PROFILE_TEST_DEBUG: "0",
    },
    windowsHide: true,
    stdio: ["ignore", file, file],
  });
  const record = {
    command: [program, ...args],
    cwd,
    log: `${name}.log`,
    pid: child.pid,
    memory: [],
  };
  results.commands.push(record);
  let sampling = false,
    refused = false;
  const sample = () => {
    if (sampling || child.exitCode !== null) return;
    sampling = true;
    try {
      const value = memory(child.pid);
      if (value) {
        record.memory.push({ ms: performance.now() - start, ...value });
        if (value.freeCommit < 536870912 && program === "npm") {
          refused = true;
          spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
            windowsHide: true,
          });
        }
      }
    } finally {
      sampling = false;
    }
  };
  const timer = setInterval(sample, 2000);
  sample();
  const exit = await new Promise((resolve, reject) => {
    child.once("exit", resolve);
    child.once("error", reject);
  });
  clearInterval(timer);
  fs.closeSync(file);
  record.exit = exit;
  record.durationMs = performance.now() - start;
  record.status = refused
    ? "REFUSED_MEMORY_RESERVE"
    : exit === 0
      ? "passed"
      : "failed";
  fs.writeFileSync(
    path.join(output, `${mode}-results.json`),
    JSON.stringify(results, null, 2),
  );
  console.log(
    `${record.status}: ${program} ${args.join(" ")} (${Math.round(record.durationMs)}ms)`,
  );
  if (exit !== 0) {
    process.exitCode = 1;
    break;
  }
}
results.finished = new Date().toISOString();
fs.writeFileSync(
  path.join(output, `${mode}-results.json`),
  JSON.stringify(results, null, 2),
);
