import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { createHash } from "node:crypto";
import net from "node:net";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { request } = require("playwright");
const WebSocket = require("ws");
const output = path.join(root, ".factory/v2/p06");
await fs.mkdir(output, { recursive: true });
const binary =
  process.env.ASF_P06_BINARY ??
  path.join(
    output,
    "run",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const profiles = JSON.parse(
  (
    await fs.readFile(
      process.env.ASF_IDE_TEST_TOOLS ??
        path.join(output, "tools/profiles.json"),
      "utf8",
    )
  ).replace(/^\uFEFF/, ""),
);
const fixture = await fs.mkdtemp(path.join(output, "benchmark-project-"));
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p06-benchmark-")),
  "registry.sqlite3",
);
const results = {
  schema: 1,
  binary,
  binary_sha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  started: new Date().toISOString(),
  machine: {
    platform: process.platform,
    release: os.release(),
    cpu: os.cpus()[0].model,
    logical_cpus: os.cpus().length,
    memory: os.totalmem(),
  },
  fixture,
  samples: [],
  memory: [],
  errors: [],
  limitations: [
    "Debug builds; OS caches are retained between process launches.",
    "Backend lifecycle latency differs from browser contribution/render latency.",
  ],
};
const run = (program, args, cwd = fixture, env = process.env) => {
  const result = spawnSync(program, args, {
    cwd,
    env,
    windowsHide: true,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
};
run("git", ["init", "-q"], fixture);
run(binary, ["init"], fixture);
await fs.mkdir(path.join(fixture, "src"));
const sources = {
  "main.ts":
    "export function add(a: number, b: number) { return a + b; }\nconst value: string = add(1, 2);\n",
  "main.py":
    "def add(a: int, b: int) -> int:\n    return a + b\n\nvalue: str = add(1, 2)\n",
  "src/main.rs":
    'fn add(a: i32,b: i32)->i32 {a+b}\nfn main() {\n let value=add(40,2);\n println!("{value}");\n}\n',
  "debug.py": "value = 40\nvalue += 2\nprint(value)\n",
  "debug.js": "let value=40;\nvalue+=2;\nconsole.log(value);\n",
};
for (const [name, text] of Object.entries(sources))
  await fs.writeFile(path.join(fixture, name), text);
await fs.writeFile(
  path.join(fixture, "Cargo.toml"),
  "[workspace]\n\n[package]\nname='asf_p06_benchmark'\nversion='0.1.0'\nedition='2021'\n",
);
run("cargo", ["build", "--quiet"], fixture, {
  ...process.env,
  CARGO_TARGET_DIR: path.join(fixture, "target"),
  CARGO_PROFILE_DEV_DEBUG: "2",
  CARGO_INCREMENTAL: "0",
});
const port = await new Promise((resolve) => {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1", () => {
    const port = socket.address().port;
    socket.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
let child, context, project, rootId, token;
async function until(action, label, timeout = 30000) {
  const start = performance.now();
  while (performance.now() - start < timeout) {
    const result = await action();
    if (result) return result;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`Benchmark deadline: ${label}`);
}
async function get(suffix) {
  const response = await context.get(
    `${base}/api/projects/${project}/${suffix}`,
  );
  assert.ok(response.ok(), await response.text());
  return response.json();
}
async function post(suffix, data = {}) {
  const response = await context.post(
    `${base}/api/projects/${project}/${suffix}`,
    {
      data,
      headers: { Origin: base, "X-Factory-CSRF": token },
      timeout: 125000,
    },
  );
  assert.ok(response.ok(), `${suffix}: ${await response.text()}`);
  return response.json();
}
async function measure(name, language, sample, action) {
  const start = performance.now();
  const result = await action();
  results.samples.push({
    name,
    language,
    sample,
    ms: performance.now() - start,
  });
  return result;
}
function memory(pid, label) {
  if (process.platform !== "win32") return;
  const script = `$items=Get-CimInstance Win32_Process; $owned=[System.Collections.Generic.HashSet[int]]::new(); [void]$owned.Add(${pid}); for($i=0;$i -lt 8;$i++){foreach($item in $items){if($owned.Contains([int]$item.ParentProcessId)){[void]$owned.Add([int]$item.ProcessId)}}}; $processes=@(foreach($idValue in $owned){Get-Process -Id $idValue -ErrorAction SilentlyContinue}); @{root=${pid}; rss=($processes|Measure-Object WorkingSet64 -Sum).Sum; privateBytes=($processes|Measure-Object PrivateMemorySize64 -Sum).Sum; processes=@($processes|ForEach-Object{@{id=$_.Id;start=$_.StartTime.ToUniversalTime().ToString('o')}})}|ConvertTo-Json -Depth 5 -Compress`;
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-Command", script],
    { encoding: "utf8", windowsHide: true },
  );
  try {
    results.memory.push({ label, ...JSON.parse(result.stdout) });
  } catch {
    results.memory.push({ label, unavailable: true });
  }
}
const file = (name) => ({ project_id: project, root_id: rootId, path: name });
try {
  const log = await fs.open(path.join(output, "benchmark-server.log"), "a");
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
      fixture,
    ],
    { cwd: fixture, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
  );
  await log.close();
  await until(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  }, "server start");
  context = await request.newContext();
  const auth = await context.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(auth.ok());
  token = (await auth.json()).csrfToken;
  project = (await (await context.get(`${base}/api/projects`)).json())[0]
    .project_id;
  rootId = (await get("files/roots")).roots.find((root) => !root.read_only).id;
  for (const profile of profiles)
    await post("ide/profiles", { profile, confirmed: true });
  for (const [language, name, line, position] of [
    ["typescript", "main.ts", 1, { line: 1, character: 24 }],
    ["python", "main.py", 3, { line: 3, character: 14 }],
    ["rust", "src/main.rs", 2, { line: 2, character: 12 }],
  ]) {
    for (let sample = 0; sample < 3; sample++) {
      const server = await measure("lsp_process_start", language, sample, () =>
        post("ide/lsp/start", {
          root_id: rootId,
          profile_id: language,
          client_id: "benchmark",
          confirmed: true,
        }),
      );
      const reused = await measure("lsp_live_reuse", language, sample, () =>
        post("ide/lsp/start", {
          root_id: rootId,
          profile_id: language,
          client_id: "benchmark",
          confirmed: true,
        }),
      );
      assert.equal(reused.id, server.id);
      const snapshot = await get(
        `files/content?root_id=${rootId}&path=${encodeURIComponent(name)}`,
      );
      let version = 1;
      const sync = async (text) =>
        post(`ide/lsp/${server.id}/sync`, {
          document: {
            file: file(name),
            base_revision: snapshot.revision,
            version: version++,
            text,
            language_id: language,
          },
          saved: false,
        });
      const synced = await sync(sources[name]);
      const call = (method, extra = {}) =>
        post(`ide/lsp/${server.id}/request`, {
          method,
          params: { textDocument: { uri: synced.uri }, position, ...extra },
          confirmed: false,
        });
      await until(
        async () => {
          try {
            return !!(await call("textDocument/hover"))?.contents;
          } catch {
            return false;
          }
        },
        "semantic readiness",
        60000,
      );
      await measure("lsp_hover", language, sample, () =>
        call("textDocument/hover"),
      );
      await measure("lsp_completion", language, sample, () =>
        call("textDocument/completion"),
      );
      await measure("lsp_definition", language, sample, () =>
        call("textDocument/definition"),
      );
      if (language !== "rust") {
        const start = performance.now();
        const fixed = sources[name]
          .replace("value: string", "value: number")
          .replace("value: str", "value: int");
        const cursor = (await get(`ide/lsp/${server.id}/events?cursor=0`))
          .cursor;
        await sync(fixed);
        await until(async () => {
          const events = await get(
            `ide/lsp/${server.id}/events?cursor=${cursor}`,
          );
          return events.events.some(
            (event) =>
              event.message.method === "textDocument/publishDiagnostics" &&
              event.message.params.diagnostics.every(
                (diagnostic) => diagnostic.severity !== 1,
              ),
          );
        }, "updated diagnostics");
        results.samples.push({
          name: "diagnostics_update",
          language,
          sample,
          ms: performance.now() - start,
        });
      }
      const prefix = language === "python" ? "#" : "//";
      const long =
        sources[name] + `${prefix} bounded benchmark line\n`.repeat(10000);
      await measure("long_document_sync_ack", language, sample, () =>
        sync(long),
      );
      await measure("long_document_semantic_roundtrip", language, sample, () =>
        call("textDocument/hover"),
      );
      memory(server.pid, `${language}-${sample}`);
      await measure("lsp_shutdown", language, sample, () =>
        post(`ide/lsp/${server.id}/stop`),
      );
    }
  }
  const configurations = [
    { id: "debugpy", path: "debug.py", source: "debug.py", line: 2 },
    { id: "js-debug", path: "debug.js", source: "debug.js", line: 2 },
    {
      id: "codelldb",
      path: `target/debug/asf_p06_benchmark${process.platform === "win32" ? ".exe" : ""}`,
      source: "src/main.rs",
      line: 3,
    },
  ];
  for (const configuration of configurations) {
    await post("ide/configurations", {
      configuration: {
        id: configuration.id,
        name: configuration.id,
        root_id: rootId,
        profile_id: configuration.id,
        program: file(configuration.path),
        arguments: { stopOnEntry: false },
      },
      confirmed: true,
    });
    for (let sample = 0; sample < 3; sample++) {
      const session = await measure(
        "dap_launch_initialized_configured",
        configuration.id,
        sample,
        () =>
          post("ide/dap/start", {
            configuration_id: configuration.id,
            breakpoints: [
              {
                file: file(configuration.source),
                breakpoints: [{ line: configuration.line }],
              },
            ],
            confirmed: true,
          }),
      );
      const stopped = await until(async () => {
        const page = await get(`ide/dap/${session.id}/events?cursor=0`);
        return page.events.find((event) => event.message.event === "stopped");
      }, "debug breakpoint");
      await measure("dap_breakpoint_roundtrip", configuration.id, sample, () =>
        post(`ide/dap/${session.id}/breakpoints`, {
          file: file(configuration.source),
          breakpoints: [{ line: configuration.line }],
        }),
      );
      const stack = await measure("dap_stack", configuration.id, sample, () =>
        post(`ide/dap/${session.id}/request`, {
          command: "stackTrace",
          arguments: { threadId: stopped.message.body.threadId },
        }),
      );
      assert.ok(stack.stackFrames.length);
      memory(session.pid, `${configuration.id}-${sample}`);
      await measure("dap_termination", configuration.id, sample, () =>
        post(`ide/dap/${session.id}/stop`),
      );
    }
  }
  const shell = profiles.find((profile) => profile.kind === "shell");
  const cookie = (await context.storageState()).cookies
    .map((cookie) => `${cookie.name}=${cookie.value}`)
    .join("; ");
  async function socket(id) {
    const ws = new WebSocket(
      `${base.replace("http", "ws")}/api/projects/${project}/ide/terminals/${id}/socket`,
      { headers: { Cookie: cookie, Origin: base } },
    );
    let text = "",
      meta,
      lastCursorQuery = -1;
    ws.on("message", (data, binary) => {
      if (binary) {
        const chunk = data.toString("utf8");
        text += chunk;
        const query = text.lastIndexOf("\x1b[6n");
        if (query > lastCursorQuery) {
          lastCursorQuery = query;
          ws.send(JSON.stringify({ type: "input", data: "\x1b[1;1R" }));
        }
      } else meta = JSON.parse(data.toString());
    });
    await new Promise((resolve, reject) => {
      ws.once("open", resolve);
      ws.once("error", reject);
    });
    return { ws, text: () => text, meta: () => meta };
  }
  async function closeSocket(ws) {
    if (ws.readyState === WebSocket.CLOSED) return;
    await new Promise((resolve) => {
      const timer = setTimeout(() => ws.terminate(), 5000);
      ws.once("close", () => {
        clearTimeout(timer);
        resolve();
      });
      ws.close();
    });
  }
  for (let sample = 0; sample < 3; sample++) {
    const terminal = await measure(
      "terminal_process_start",
      shell.id,
      sample,
      () =>
        post("ide/terminals/start", {
          root_id: rootId,
          profile_id: shell.id,
          confirmed: true,
          cols: 100,
          rows: 28,
        }),
    );
    memory(terminal.pid, `terminal-${sample}`);
    const first = await socket(terminal.id);
    const start = performance.now();
    const command =
      process.platform === "win32"
        ? "[Console]::Write(('x'*524288)); [Console]::WriteLine(); Write-Output ('__BENCH_'+'END__')\r"
        : "printf '%524288s' '' | tr ' ' x; printf '\\n__BENCH_%s\\n' 'END__'\n";
    first.ws.send(JSON.stringify({ type: "input", data: command }));
    let receiver = first,
      evictions = 0;
    try {
      for (let attempt = 0; attempt < 4; attempt++) {
        await until(
          () =>
            receiver.text().includes("__BENCH_END__") ||
            receiver.meta()?.overflowed === true,
          "terminal completion or explicit backpressure",
          30000,
        );
        if (receiver.text().includes("__BENCH_END__")) break;
        evictions++;
        await closeSocket(receiver.ws);
        receiver = await socket(terminal.id);
      }
      assert.ok(
        receiver.text().includes("__BENCH_END__"),
        "bounded reconnect attempts did not recover completion",
      );
    } catch (error) {
      results.terminal_failure = {
        metadata: receiver.meta(),
        observed_vt_bytes: Buffer.byteLength(receiver.text()),
        tail: receiver.text().slice(-4096),
        initial_metadata: first.meta(),
      };
      throw error;
    }
    const elapsed = performance.now() - start;
    results.samples.push({
      name: "terminal_512k_request_to_marker",
      language: shell.id,
      sample,
      ms: elapsed,
      requested_ascii_characters: 524288,
      initial_live_vt_bytes: Buffer.byteLength(first.text()),
      recovered_or_live_vt_bytes: Buffer.byteLength(receiver.text()),
      subscriber_evictions: evictions,
      initial_live_vt_bytes_per_second:
        Buffer.byteLength(first.text()) / (elapsed / 1000),
    });
    receiver.ws.send(JSON.stringify({ type: "resize", cols: 96, rows: 32 }));
    await closeSocket(receiver.ws);
    const reconnected = await measure(
      "terminal_reconnect_snapshot",
      shell.id,
      sample,
      async () => {
        const current = await socket(terminal.id);
        await until(
          () => current.text().includes("__BENCH_END__"),
          "terminal replay",
        );
        return current;
      },
    );
    assert.ok(reconnected.meta()?.bounded);
    await closeSocket(reconnected.ws);
    await measure("terminal_termination", shell.id, sample, () =>
      post(`ide/terminals/${terminal.id}/stop`),
    );
  }
  for (const name of ["p06-editor", "p06-workspace"]) {
    const manifest = JSON.parse(
      await fs.readFile(
        path.join(root, `extensions/${name}/extension.json`),
        "utf8",
      ),
    );
    await post("ide/extensions/install", { manifest, trusted: true });
    for (let sample = 0; sample < 3; sample++) {
      await measure("extension_enable_backend", name, sample, () =>
        post(`ide/extensions/${manifest.id}/enable`),
      );
      await measure("extension_disable_backend", name, sample, () =>
        post(`ide/extensions/${manifest.id}/disable`),
      );
    }
    await post(`ide/extensions/${manifest.id}/uninstall`);
  }
  const state = await get("ide/state");
  assert.equal(state.servers.filter((server) => server.alive).length, 0);
  assert.equal(
    state.debug_sessions.filter((session) => session.alive).length,
    0,
  );
  memory(child.pid, "asf-after-service-cycles");
  if (process.platform === "win32") {
    await new Promise((resolve) => setTimeout(resolve, 300));
    const recorded = results.memory
      .filter((item) => item.label !== "asf-after-service-cycles")
      .flatMap((item) => item.processes ?? []);
    const ids = [...new Set(recorded.map((item) => item.id))];
    const script = `@(${ids.join(",")})|ForEach-Object{Get-Process -Id $_ -ErrorAction SilentlyContinue}|ForEach-Object{@{id=$_.Id;start=$_.StartTime.ToUniversalTime().ToString('o')}}|ConvertTo-Json -Compress`;
    const checked = spawnSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      { encoding: "utf8", windowsHide: true },
    );
    assert.equal(checked.status, 0, checked.stderr);
    const active = checked.stdout.trim() ? JSON.parse(checked.stdout) : [];
    results.remaining_owned_tool_processes = [active]
      .flat()
      .filter((item) =>
        recorded.some(
          (original) =>
            original.id === item.id && original.start === item.start,
        ),
      );
    assert.equal(
      results.remaining_owned_tool_processes.length,
      0,
      "owned tool processes remained after termination",
    );
  }
} catch (error) {
  results.errors.push(error.stack);
  console.error(error.stack);
  process.exitCode = 1;
} finally {
  await context?.dispose();
  if (child?.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
  const groups = new Map();
  for (const sample of results.samples) {
    const key = `${sample.name}:${sample.language}`;
    groups.set(key, [...(groups.get(key) ?? []), sample.ms]);
  }
  results.summary = [...groups].map(([key, values]) => {
    values.sort((a, b) => a - b);
    return {
      key,
      n: values.length,
      median_ms: values[Math.floor(values.length / 2)],
      min_ms: values[0],
      max_ms: values.at(-1),
    };
  });
  results.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "benchmarks.json"),
    JSON.stringify(results, null, 2),
  );
}
