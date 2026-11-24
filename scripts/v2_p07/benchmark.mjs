import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p07"),
  binary =
    process.env.ASF_P07_BINARY ??
    path.join(
      output,
      "run",
      process.platform === "win32" ? "factory.exe" : "factory",
    );
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const fixture = await fs.mkdtemp(path.join(output, "benchmark-"));
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p07-bench-")),
  "registry.sqlite3",
);
const result = {
  schema: 1,
  started: new Date().toISOString(),
  platform: process.platform,
  cpu: os.cpus()[0]?.model,
  logicalCpus: os.cpus().length,
  ram: os.totalmem(),
  fixture,
  registry,
  measurements: [],
  memory: [],
  database: [],
  notes: [
    "Debug binary; OS filesystem caches retained. Controlled process agent, not model-inference timings.",
  ],
};
const run = (program, args, cwd = root) => {
  const r = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
};
result.sourceCommit = run("git", ["rev-parse", "HEAD"]).trim();
result.binarySha256 = createHash("sha256")
  .update(await fs.readFile(binary))
  .digest("hex");
result.scriptSha256 = createHash("sha256")
  .update(await fs.readFile(fileURLToPath(import.meta.url)))
  .digest("hex");
const agent = path.join(
  fixture,
  process.platform === "win32" ? "controlled.exe" : "controlled",
);
run("rustc", [
  "--edition=2021",
  path.join(
    root,
    "crates/factory-conversation/tests/support/controlled_agent.rs",
  ),
  "-o",
  agent,
]);
const project = path.join(fixture, "Project");
await fs.mkdir(project);
run("git", ["init", "-q", "-b", "main"], project);
run(binary, ["init"], project);
await fs.writeFile(path.join(project, ".gitignore"), ".factory/\n");
await fs.writeFile(path.join(project, "source.txt"), "benchmark\n");
run("git", ["add", "."], project);
run(
  "git",
  [
    "-c",
    "user.name=P07 Benchmark",
    "-c",
    "user.email=benchmark@example.invalid",
    "commit",
    "-qm",
    "fixture",
  ],
  project,
);
await fs.writeFile(
  path.join(project, ".factory/config.toml"),
  ["planner", "worker", "reviewer"]
    .map(
      (role) =>
        `[agents.${role}]\nkind="custom"\ncommand=${JSON.stringify(agent)}\nargs=["${role}"]\n[[role_assignments]]\nrole="${role}"\nagent="${role}"\npreferred=true\n`,
    )
    .join("\n"),
);
const port = await new Promise((resolve) => {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1", () => {
    const port = socket.address().port;
    socket.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
let child, browser, context, page, id;
const record = (metric, ms, extra = {}) =>
  result.measurements.push({ metric, ms, ...extra });
async function eventually(fn, timeout = 20000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const value = await fn();
    if (value) return value;
    await new Promise((r) => setTimeout(r, 20));
  }
  throw new Error("Benchmark condition timed out");
}
async function auth() {
  const r = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(r.ok());
  return (await r.json()).csrfToken;
}
async function mutation(suffix, body) {
  const csrf = await auth();
  const start = performance.now();
  const r = await context.request.post(`${base}/api/projects/${id}/${suffix}`, {
    headers: { Origin: base, "X-Factory-CSRF": csrf },
    data: body,
  });
  assert.ok(r.ok(), await r.text());
  return { value: await r.json(), ms: performance.now() - start };
}
async function read(suffix) {
  const r = await context.request.get(`${base}/api/projects/${id}/${suffix}`);
  assert.ok(r.ok(), await r.text());
  return r.json();
}
async function dbSize(label) {
  const entries = [];
  for (const name of ["db.sqlite3", "db.sqlite3-wal", "db.sqlite3-shm"]) {
    try {
      entries.push({
        name,
        bytes: (await fs.stat(path.join(project, ".factory", name))).size,
      });
    } catch {}
  }
  result.database.push({ label, entries });
}
function memory(label) {
  if (process.platform !== "win32") return;
  const data = JSON.parse(
    run("powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `Get-Process -Id ${child.pid} | Select-Object PrivateMemorySize64,WorkingSet64 | ConvertTo-Json -Compress`,
    ]),
  );
  result.memory.push({ label, ...data });
}
try {
  const log = await fs.open(path.join(output, "benchmark-server.log"), "a");
  const start = performance.now();
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
    { cwd: project, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
  );
  await log.close();
  await eventually(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  });
  record("server_health", performance.now() - start);
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
  });
  await auth();
  id = (await (await context.request.get(`${base}/api/projects`)).json())[0]
    .project_id;
  await dbSize("empty");
  memory("idle");
  for (let i = 0; i < 100; i++) {
    const { ms } = await mutation("conversations", {
      title: `Historical conversation ${i}`,
    });
    record("conversation_create", ms);
  }
  const conversation = (
    await mutation("conversations", {
      title: "200-message rendering benchmark",
    })
  ).value.id;
  const code =
    "```rust\n" +
    "let value = 42; // bounded long content 🦀\n".repeat(30) +
    "```\n";
  for (let i = 0; i < 100; i++) {
    const accepted = await mutation(`conversations/${conversation}/messages`, {
      key: `benchmark-${i}`,
      workspace_session_id: `s_${"a".repeat(32)}`,
      content: `Question ${i}: explain this supplied text without actions.\n${code}`,
      context: [],
    });
    record("message_acceptance", accepted.ms);
    const began = performance.now();
    const response = await eventually(async () => {
      const messages = await read(`conversations/${conversation}/messages`);
      return messages.find(
        (m) =>
          m.id === accepted.value.response_id &&
          ["finished", "failed"].includes(m.status),
      );
    });
    assert.equal(response.status, "finished", response.content);
    record("controlled_response", performance.now() - began);
  }
  for (let i = 0; i < 5; i++) {
    const start = performance.now();
    const messages = await read(
      `conversations/${conversation}/messages?limit=100`,
    );
    record("open_latest_100", performance.now() - start, {
      bytes: Buffer.byteLength(JSON.stringify(messages)),
    });
    assert.equal(messages.length, 100);
  }
  await dbSize("200_messages_101_conversations");
  memory("after_messages");
  for (let i = 0; i < 3; i++) {
    const cid = (
      await mutation("conversations", { title: `SSE measurement ${i}` })
    ).value.id;
    const controller = new AbortController();
    const cookie = (await context.cookies())
      .map((c) => `${c.name}=${c.value}`)
      .join("; ");
    const response = await fetch(
      `${base}/api/projects/${id}/conversations/${cid}/events`,
      { headers: { Cookie: cookie, Origin: base }, signal: controller.signal },
    );
    const reader = response.body.getReader();
    const start = performance.now();
    const received = (async () => {
      let text = "";
      while (true) {
        const { value, done } = await reader.read();
        if (done) throw new Error("Stream closed before accepted event");
        text += new TextDecoder().decode(value);
        if (text.includes("conversation.message.accepted"))
          return performance.now() - start;
      }
    })();
    const accepted = await mutation(`conversations/${cid}/messages`, {
      key: "event",
      workspace_session_id: `s_${"a".repeat(32)}`,
      content: "Question",
      context: [],
    });
    record("accept_to_sse_observed", await received, {
      httpAcknowledgment: accepted.ms,
    });
    controller.abort();
  }
  page = await context.newPage();
  const render = performance.now();
  await page.goto(`${base}/workspace.html`);
  await page.getByLabel("Open registered project").selectOption(id);
  await page.locator(`#project-${id}[aria-selected=true]`).waitFor();
  const panel = page.locator(".chat-panel:visible").first();
  await panel
    .getByLabel("Conversation", { exact: true })
    .selectOption(conversation);
  await eventually(
    async () => (await panel.locator(".chat-message").count()) >= 50,
  );
  record("workspace_and_latest_50_render", performance.now() - render);
  for (let i = 0; i < 3; i++) {
    const start = performance.now();
    await panel.getByRole("button", { name: /Load earlier messages/ }).click();
    await eventually(
      async () =>
        (await panel.locator(".chat-message").count()) >= (i + 2) * 50,
    );
    record("append_50_render", performance.now() - start);
  }
  assert.equal(await panel.locator(".chat-message").count(), 200);
  await page.screenshot({
    path: path.join(output, "screenshots", "200-message-benchmark.png"),
  });
  const beside = performance.now();
  await panel.getByRole("button", { name: "Open beside", exact: true }).click();
  await eventually(
    async () => (await page.locator(".chat-panel:visible").count()) === 2,
  );
  await eventually(
    async () =>
      (await page.locator(".chat-panel:visible .chat-message").count()) >= 250,
  );
  record("second_conversation_panel_render", performance.now() - beside);
  const secondary = page.locator(".chat-panel:visible").nth(1);
  const otherId = await secondary
    .getByLabel("Conversation", { exact: true })
    .locator("option")
    .evaluateAll(
      (options, current) =>
        options.find((option) => option.value && option.value !== current)
          ?.value,
      conversation,
    );
  assert.ok(otherId);
  const different = performance.now();
  await secondary
    .getByLabel("Conversation", { exact: true })
    .selectOption(otherId);
  await eventually(
    async () => (await secondary.locator(".chat-message").count()) === 2,
  );
  assert.equal(
    await panel.getByLabel("Conversation", { exact: true }).inputValue(),
    conversation,
  );
  assert.equal(await panel.locator(".chat-message").count(), 200);
  record(
    "different_conversation_in_second_panel",
    performance.now() - different,
    { primary: conversation, secondary: otherId },
  );
  await page.screenshot({
    path: path.join(output, "screenshots", "multiple-conversation-panels.png"),
  });
  for (let i = 0; i < 3; i++) {
    const reload = performance.now();
    await page.reload();
    await page.locator(".chat-panel:visible .chat-message").first().waitFor();
    record("workspace_reload_to_messages", performance.now() - reload);
  }
  result.browserHeap = await page.evaluate(
    () => performance.memory?.usedJSHeapSize ?? null,
  );
  result.browser = browser.version();
  memory("browser_open");
  result.finished = new Date().toISOString();
} finally {
  await fs.writeFile(
    path.join(output, "benchmarks.json"),
    JSON.stringify(result, null, 2),
  );
  await context?.close();
  await browser?.close();
  if (child && child.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
}
