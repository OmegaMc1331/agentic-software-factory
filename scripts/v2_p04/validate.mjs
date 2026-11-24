import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p04");
fs.mkdirSync(output, { recursive: true });
const mode = process.argv[2];
const workers = process.env.ASF_TEST_WORKERS;
if (workers && !/^[1-8]$/.test(workers))
  throw new Error("ASF_TEST_WORKERS must be between 1 and 8.");
if (!["rust", "frontend", "packaging", "tooling"].includes(mode))
  throw new Error("Expected rust, frontend, packaging or tooling.");
const jobs =
  mode === "rust"
    ? [
        ["fmt", "cargo", ["fmt", "--all", "--", "--check"]],
        [
          "clippy",
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
        [
          "p03-p04-targeted",
          "cargo",
          [
            "test",
            "-p",
            "factory-api",
            "-p",
            "factory-projects",
            "--test",
            "multi_project",
            "--test",
            "projects",
            "--test",
            "workspaces",
            "-j",
            "1",
            "--",
            "--test-threads=1",
          ],
        ],
        [
          "rust-serial",
          "cargo",
          ["test", "--workspace", "-j", "1", "--", "--test-threads=1"],
        ],
        ["rust-parallel", "cargo", ["test", "--workspace", "-j", "1"]],
      ]
    : mode === "tooling"
      ? [
          [
            "tooling",
            process.env.PYTHON ?? "python",
            ["-B", "scripts/test_v2_baseline.py"],
          ],
          [
            "inventory",
            process.env.PYTHON ?? "python",
            [
              "-B",
              "scripts/v2_baseline.py",
              "inventory",
              "--check",
              "--output",
              ".factory/v2/p04/inventory.json",
            ],
          ],
        ]
      : mode === "packaging"
        ? [
            [
              "embedded-test",
              "cargo",
              [
                "test",
                "-p",
                "factory-api",
                "--features",
                "embedded-dashboard",
                "--lib",
                "projects_assets_and_font_licenses_are_embedded",
                "-j",
                "1",
              ],
            ],
            [
              "embedded-build",
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
          ]
        : ["format:check", "lint", "typecheck", "test", "build"].map(
            (label) => [
              label,
              "npm",
              label === "test"
                ? [
                    "test",
                    ...(workers ? ["--", `--maxWorkers=${workers}`] : []),
                  ]
                : ["run", label],
            ],
          );
const records = [];
const sources = {};
for (const file of [
  "crates/factory-api/src/workspaces.rs",
  "crates/factory-api/src/application.rs",
  "crates/factory-api/src/dashboard.rs",
  "crates/factory-api/tests/workspaces.rs",
  "apps/dashboard/src/api.ts",
]) {
  sources[file] = createHash("sha256")
    .update(
      fs.readFileSync(path.join(root, file), "utf8").replaceAll("\r\n", "\n"),
    )
    .digest("hex");
}
for (const [label, program, args] of jobs) {
  const start = performance.now(),
    log = fs.openSync(
      path.join(output, `${mode}-${label.replaceAll(":", "-")}.log`),
      "w",
    );
  const result = spawnSync(program, args, {
    cwd: mode === "frontend" ? path.join(root, "apps/dashboard") : root,
    env: process.env,
    windowsHide: true,
    shell: program === "npm" && process.platform === "win32",
    stdio: ["ignore", log, log],
  });
  fs.closeSync(log);
  records.push({
    label,
    command: [program, ...args].join(" "),
    exit: result.status,
    ms: performance.now() - start,
    error: result.error?.message,
  });
  fs.writeFileSync(
    path.join(output, `${mode}-gates.json`),
    JSON.stringify(
      { started: new Date().toISOString(), sources, records },
      null,
      2,
    ) + "\n",
  );
  console.log(JSON.stringify(records.at(-1)));
  if (result.status !== 0) {
    process.exitCode = 1;
    break;
  }
}
