import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p06/tools");
await fs.mkdir(output, { recursive: true });
const read = async (file) =>
  JSON.parse((await fs.readFile(file, "utf8")).replace(/^\uFEFF/, ""));
const p01 = await read(path.join(root, ".factory/v2/p01/config.json")).catch(
  () => ({}),
);
const ra = spawnSync("rustup", ["which", "rust-analyzer"], {
  encoding: "utf8",
  windowsHide: true,
});
const definitions = [
  {
    id: "typescript",
    name: "TypeScript",
    kind: "lsp",
    executable: process.env.ASF_P06_NODE ?? p01.node,
    arguments: [process.env.ASF_P06_TYPESCRIPT_SERVER ?? p01.lsp, "--stdio"],
    languages: [
      "typescript",
      "typescriptreact",
      "javascript",
      "javascriptreact",
    ],
  },
  {
    id: "python",
    name: "Pyright",
    kind: "lsp",
    executable: process.env.ASF_P06_NODE ?? p01.node,
    arguments: [process.env.ASF_P06_PYRIGHT ?? p01.pyright, "--stdio"],
    languages: ["python"],
  },
  {
    id: "rust",
    name: "rust-analyzer",
    kind: "lsp",
    executable:
      process.env.ASF_P06_RUST_ANALYZER ??
      (ra.status === 0 ? ra.stdout.trim() : undefined),
    arguments: [],
    languages: ["rust"],
    initialization_options: {
      cargo: { buildScripts: { enable: false } },
      procMacro: { enable: false },
      checkOnSave: false,
    },
  },
  {
    id: "debugpy",
    name: "Python debugpy",
    kind: "dap",
    executable: process.env.ASF_P06_PYTHON ?? p01.python,
    arguments: ["-m", "debugpy.adapter"],
    languages: ["python"],
  },
  {
    id: "js-debug",
    name: "JavaScript debugger",
    kind: "dap",
    transport: "tcp",
    executable: process.env.ASF_P06_NODE ?? p01.node,
    arguments: [
      process.env.ASF_P06_JS_DEBUG ??
        path.join(output, "js-debug/js-debug/src/dapDebugServer.js"),
      "${port}",
      "127.0.0.1",
    ],
    languages: ["javascript", "typescript"],
    settings: {
      target_sessions: true,
      launch: {
        type: "pwa-node",
        request: "launch",
        name: "ASF JavaScript",
        autoAttachChildProcesses: false,
      },
    },
  },
  {
    id: "codelldb",
    name: "Rust CodeLLDB",
    kind: "dap",
    transport: "tcp",
    executable:
      process.env.ASF_P06_CODELLDB ??
      path.join(
        output,
        `codelldb/extension/adapter/codelldb${process.platform === "win32" ? ".exe" : ""}`,
      ),
    arguments: ["--port", "${port}"],
    languages: ["rust"],
  },
  {
    id: process.platform === "win32" ? "powershell" : "shell",
    name: process.platform === "win32" ? "PowerShell" : "Development shell",
    kind: "shell",
    executable:
      process.env.ASF_P06_SHELL ??
      (process.platform === "win32"
        ? path.join(
            process.env.SystemRoot,
            "System32/WindowsPowerShell/v1.0/powershell.exe",
          )
        : (process.env.SHELL ?? "/bin/bash")),
    arguments: process.platform === "win32" ? ["-NoLogo", "-NoProfile"] : [],
    languages: [],
  },
];
const evidence = {
  schema: 1,
  date: new Date().toISOString(),
  platform: process.platform,
  tools: [],
  unavailable: [],
};
const profiles = [];
for (const definition of definitions) {
  const paths = [
    definition.executable,
    ...definition.arguments.filter(
      (argument) => argument && path.isAbsolute(argument),
    ),
  ];
  const present =
    definition.executable &&
    definition.arguments.every((argument) => typeof argument === "string") &&
    (await Promise.all(
      paths.map((file) =>
        fs
          .stat(file)
          .then((stat) => stat.isFile())
          .catch(() => false),
      ),
    ).then((values) => values.every(Boolean)));
  if (!present) {
    evidence.unavailable.push(definition.id);
    continue;
  }
  const fingerprints = [];
  for (const file of paths)
    fingerprints.push({
      path: file,
      sha256: createHash("sha256")
        .update(await fs.readFile(file))
        .digest("hex"),
    });
  profiles.push({ ...definition, timeout_ms: 60000, enabled: true });
  evidence.tools.push({ id: definition.id, files: fingerprints });
}
await fs.writeFile(
  path.join(output, "profiles.json"),
  JSON.stringify(profiles, null, 2),
);
await fs.writeFile(
  path.join(output, "tool-provenance.json"),
  JSON.stringify(evidence, null, 2),
);
console.log(
  JSON.stringify(
    {
      profiles: profiles.map((profile) => profile.id),
      unavailable: evidence.unavailable,
      configuration: path.join(output, "profiles.json"),
    },
    null,
    2,
  ),
);
if (evidence.unavailable.length) process.exitCode = 1;
