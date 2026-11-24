import fs from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output =
  process.env.ASF_VALIDATION_OUTPUT ?? path.join(root, ".factory/v2/p11/logs");
fs.mkdirSync(output, { recursive: true });
const mode = process.argv[2] ?? "targeted";
const runId = Date.now();
const commands = {
  memory_resume: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-runtime",
        "--test",
        "lifecycle",
        "p11_resume",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  memory: [
    [
      "cargo",
      [
        "test",
        "--no-fail-fast",
        "-p",
        "factory-memory",
        "-p",
        "factory-core",
        "--test",
        "memory_context",
        "-p",
        "factory-api",
        "--test",
        "memory",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  memory_lib: [
    [
      "cargo",
      ["test", "-p", "factory-memory", "-j", "1", "--", "--test-threads=1"],
    ],
  ],

  lifecycle: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-db",
        "--test",
        "lifecycle",
        "-p",
        "factory-runtime",
        "--test",
        "lifecycle",
        "-p",
        "factory-api",
        "--test",
        "lifecycle",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-api",
        "--lib",
        "presence::tests",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  profiles_fast: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "agent_profiles",
        "-p",
        "factory-api",
        "--test",
        "agent_profiles",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  clippy: [
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
  ],
  profiles: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "agent_profiles",
        "--test",
        "dynamic_scheduler",
        "-p",
        "factory-api",
        "--test",
        "agent_profiles",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],

  r1: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "semantic_validation_failure",
        "-j",
        "1",
        "--",
        "--nocapture",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "policy_enforcement",
        "reported_commands_outside_the_policy",
        "-j",
        "1",
      ],
    ],
    [
      "cargo",
      ["test", "-p", "factory-core", "--lib", "authorization::", "-j", "1"],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-db",
        "scheduler::tests",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  legacy_lifecycle: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "legacy_runs_have_no_new_deadline",
        "-j",
        "1",
        "--",
        "--nocapture",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "policy_setup_failure",
        "-j",
        "1",
        "--",
        "--nocapture",
      ],
    ],
  ],
  fixes: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "exhausted_cli_capacity",
        "-j",
        "1",
        "--",
        "--nocapture",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "cancellation_drains",
        "-j",
        "1",
        "--",
        "--nocapture",
      ],
    ],
    [
      "cargo",
      ["test", "-p", "factory-core", "--lib", "authorization::", "-j", "1"],
    ],
    [
      "cargo",
      ["test", "-p", "factory-core", "--lib", "default_team_picks", "-j", "1"],
    ],
  ],
  compatibility: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-api",
        "--test",
        "multi_project",
        "--test",
        "conversations",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  core: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "-p",
        "factory-db",
        "-p",
        "factory-runtime",
        "-p",
        "factory-conversation",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  scheduler: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "dynamic_scheduler",
        "-j",
        "1",
        "--",
        "--test-threads=1",
        "--nocapture",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-db",
        "scheduler::tests",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  baseline: [
    [
      "python",
      [
        "-m",
        "unittest",
        "discover",
        "-s",
        "scripts",
        "-p",
        "test_v2_baseline.py",
      ],
    ],
  ],
  tooling: [["node", ["--test", "scripts/release-checksums.test.mjs"]]],
  permissions: [
    [
      "cargo",
      ["test", "-p", "factory-core", "--lib", "authorization::", "-j", "1"],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-core",
        "--test",
        "policy_enforcement",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
  ],
  targeted: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-conversation",
        "-p",
        "factory-db",
        "-j",
        "1",
        "--",
        "--test-threads=1",
      ],
    ],
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-api",
        "--test",
        "conversations",
        "--test",
        "files",
        "--test",
        "ide",
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
  tools: [
    [
      "cargo",
      [
        "test",
        "-p",
        "factory-ide",
        "--test",
        "services",
        "-j",
        "1",
        "--",
        "--ignored",
        "--test-threads=1",
        "--nocapture",
      ],
    ],
  ],
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
const previous = path.join(output, `${mode}-results.json`);
if (fs.existsSync(previous)) {
  fs.mkdirSync(path.join(output, "attempts"), { recursive: true });
  fs.copyFileSync(
    previous,
    path.join(output, "attempts", `${mode}-${runId}.json`),
  );
}
const sourceFingerprint = () => {
  const sourcePaths = spawnSync(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard"],
    { cwd: root, encoding: "utf8", windowsHide: true },
  )
    .stdout.trim()
    .split(/\r?\n/)
    .filter(
      (file) =>
        file.startsWith("crates/") ||
        file.startsWith("apps/dashboard/") ||
        ["Cargo.toml", "Cargo.lock", ".github/workflows/ci.yml"].includes(file),
    )
    .sort();
  const fingerprint = createHash("sha256");
  for (const file of sourcePaths) {
    fingerprint.update(file + "\0");
    fingerprint.update(fs.readFileSync(path.join(root, file)));
  }
  return fingerprint.digest("hex");
};
const results = {
  schema: 1,
  sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  }).stdout.trim(),
  sourceFingerprint: sourceFingerprint(),
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
  const name = `${mode}-${runId}-${results.commands.length + 1}`,
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
      PYTHONDONTWRITEBYTECODE: "1",
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
results.finalSourceFingerprint = sourceFingerprint();
results.sourceUnchanged =
  results.sourceFingerprint === results.finalSourceFingerprint;
if (!results.sourceUnchanged) process.exitCode = 1;
fs.writeFileSync(
  path.join(output, `${mode}-results.json`),
  JSON.stringify(results, null, 2),
);
