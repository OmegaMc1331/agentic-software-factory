import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const sourcePath = path.join(root, ".factory/v2/p11/native/results.json");
const source = JSON.parse(await fs.readFile(sourcePath, "utf8"));
const a = source.projects.A,
  b = source.projects.B;
assert.ok(
  source.timeline.some((e) => e.event === "mission.independently_verified"),
);
assert.equal(a.verification.exit, 0);
assert.equal(a.oracle.exit, 0);
assert.equal(a.adapterOracle.exit, 0);
assert.equal(source.cleanExtraction.extraction.state, "completed");
assert.equal(source.injectionExtraction.extraction.state, "completed");
assert.equal(source.acceptedCandidate.state, "accepted");
assert.ok(
  source.disabledConversation.usage[0].memories.every(
    (m) => m.memoryId !== a.memory.id,
  ),
);
assert.ok(
  source.bConversation.usage[0].memories.some(
    (m) => m.memoryId === b.memory.id,
  ),
);
const binary = process.env.ASF_P11_BINARY;
assert.ok(binary);
const output =
  process.env.ASF_P11_REPLAY ?? path.join(root, ".factory/v2/p11/replay");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
try {
  await fs.access(path.join(output, "results.json"));
  throw Error(
    "Preserve previous replay evidence; select a new output directory",
  );
} catch (e) {
  if (e.code !== "ENOENT") throw e;
}
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const result = {
  schema: 1,
  kind: "Model-free continuation of retained genuine P11 acceptance; does not replace original model evidence",
  sourceEvidenceSha256: hash(await fs.readFile(sourcePath)),
  binarySha256: hash(await fs.readFile(binary)),
  modelInvocations: 0,
  started: new Date().toISOString(),
  tests: [],
  screenshots: [],
  pageErrors: [],
};
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium, request } = require("playwright");
const port = await new Promise((resolve) => {
  const s = net.createServer();
  s.listen(0, "127.0.0.1", () => {
    const port = s.address().port;
    s.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(path.join(output, "server.log"), "w");
const server = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    path.join(source.fixture, "registry.sqlite3"),
    "--allow-project-root",
    source.fixture,
  ],
  { cwd: a.root, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
);
await log.close();
let inspector, browser, csrf;
async function until(action, label) {
  const end = Date.now() + 60000;
  while (Date.now() < end) {
    const value = await action();
    if (value) return value;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw Error(`Timed out: ${label}`);
}
try {
  await until(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  }, "owned replay startup");
  inspector = await request.newContext();
  const auth = await inspector.post(base + "/api/auth/session", {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(auth.ok());
  csrf = (await auth.json()).csrfToken;
  async function api(url, method = "GET", data) {
    const r = await inspector.fetch(base + url, {
      method,
      headers: { Origin: base, "X-Factory-CSRF": csrf },
      ...(data === undefined ? {} : { data }),
    });
    assert.ok(r.ok(), `${url}: ${await r.text()}`);
    return r.json();
  }
  const read = (p, suffix) => api(`/api/projects/${p.projectId}/${suffix}`);
  for (const p of [a, b])
    await api(`/api/projects/${p.projectId}/open`, "POST");
  const before = await read(a, `runs/${a.runId}`);
  assert.equal(before.run.status, "completed");
  async function allSessions(p) {
    const profiles = await read(p, "agent-profiles");
    const agents = [
      ...new Set([
        ...profiles.map((p) => p.id),
        "planner",
        "worker",
        "reviewer",
      ]),
    ];
    const rows = (
      await Promise.all(
        agents.map((agent) =>
          read(p, `agents/${encodeURIComponent(agent)}/sessions`),
        ),
      )
    ).flat();
    return [...new Map(rows.map((s) => [s.id, s])).values()].sort(
      (a, b) => a.id - b.id,
    );
  }
  const sessionsBefore = { A: await allSessions(a), B: await allSessions(b) };
  const workspaces = await api("/api/workspaces");
  let workspace;
  for (const entry of workspaces) {
    const doc = (await api(`/api/workspaces/${entry.id}`)).document;
    if (doc.sessions.some((s) => s.id === a.sessionId)) {
      workspace = doc;
      break;
    }
  }
  assert.ok(
    workspace,
    "Restore the owning P04 workspace, not a newly invented session",
  );
  result.workspaceId = workspace.id;
  browser = await chromium.launch({
    channel: "msedge",
    headless: true,
    args: [
      "--disable-gpu",
      "--disable-extensions",
      "--disable-background-networking",
    ],
  });
  const context = await browser.newContext({
    viewport: { width: 1500, height: 1050 },
    reducedMotion: "reduce",
  });
  await context.addInitScript(
    (id) => localStorage.setItem("asf.v2.workspace.last", id),
    workspace.id,
  );
  const page = await context.newPage();
  page.on("pageerror", (e) => result.pageErrors.push(String(e)));
  async function settled() {
    await until(
      async () =>
        (await page.locator(".ws-status").innerText()).includes(
          "Workspace saved",
        ),
      "durable workspace",
    );
  }
  async function shot(name) {
    await settled();
    const file = `screenshots/${name}.png`;
    await page.screenshot({ path: path.join(output, file) });
    result.screenshots.push({
      file,
      sha256: hash(await fs.readFile(path.join(output, file))),
    });
  }
  async function view(name) {
    await page.getByRole("button", { name: "Views", exact: true }).click();
    await page.getByRole("menuitem", { name, exact: true }).click();
  }
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").selectOption(a.projectId);
  await settled();
  await view("Orchestrator");
  const chat = page.locator(".chat-panel:visible").last();
  await chat
    .getByLabel("Conversation", { exact: true })
    .selectOption(a.conversationId);
  await until(
    async () =>
      (await chat.locator(".chat-message").count()) >= a.messages.length,
    "durable real conversation",
  );
  await shot("restored-real-conversation");
  await view("Progress");
  await page
    .getByRole("button", { name: "Open workflow and review", exact: true })
    .first()
    .click();
  await page
    .getByRole("button", { name: new RegExp(`#${a.secondWorker.id}.*success`) })
    .first()
    .click();
  const usage = page
    .getByText("Project memory supplied to this invocation", { exact: true })
    .last();
  await usage.click();
  await page
    .locator(".asf-memory-usage:visible")
    .last()
    .getByText(a.memory.id, { exact: false })
    .waitFor();
  assert.match(
    await page.locator(".asf-memory-usage:visible").last().innerText(),
    /revision 5/,
  );
  await shot("actual-worker-memory-audit");
  await page.getByRole("button", { name: "Memory", exact: true }).click();
  const panel = page.locator(".asf-memory:visible").last();
  await panel
    .getByRole("button", { name: "Saved knowledge", exact: true })
    .click();
  await panel.getByRole("button", { name: "Refresh", exact: true }).click();
  await panel.getByLabel("Search project memory", { exact: true }).fill("");
  await panel
    .getByRole("button", { name: "Search memories", exact: true })
    .click();
  await panel
    .locator(".asf-memory-list button")
    .filter({ hasText: a.memory.title })
    .click();
  const revisionToggle = panel.getByText("Revision history", { exact: true });
  if ((await revisionToggle.locator("..").getAttribute("open")) === null)
    await revisionToggle.click();
  await panel.getByText("Revision 5", { exact: false }).first().waitFor();
  await shot("genuine-memory-revision-history");
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await shot("genuine-memory-dark");
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await shot("genuine-memory-light");
  await panel
    .getByRole("button", { name: "Delete memory", exact: true })
    .click();
  const dialog = page.getByRole("alertdialog", {
    name: "Delete project memory",
    exact: true,
  });
  await dialog
    .getByRole("button", { name: "Delete permanently", exact: true })
    .click();
  await panel
    .getByText("Memory deleted from active storage and retrieval.", {
      exact: true,
    })
    .waitFor();
  const selected = await api(
    `/api/projects/${a.projectId}/memory/retrieve`,
    "POST",
    { query: "balance transfer repository adapter" },
  );
  assert.ok(!selected.records.some((m) => m.id === a.memory.id));
  result.deletedRetrieval = selected;
  await page.reload();
  await page.getByLabel("Open registered project").waitFor();
  await page.getByRole("button", { name: "Memory", exact: true }).click();
  assert.ok(!(await read(a, "memory")).items.some((m) => m.id === a.memory.id));
  await shot("deleted-memory-after-reload");
  result.tests.push({
    name: "Actual invocation audit and original revision history are visible; acknowledged deletion survives reload and retrieval",
    status: "passed",
  });
  await page.getByLabel("Open registered project").selectOption(b.projectId);
  await page.getByRole("button", { name: "Memory", exact: true }).click();
  const other = page.locator(".asf-memory:visible").last();
  await other
    .locator(".asf-memory-list button")
    .filter({ hasText: b.memory.title })
    .click();
  await other
    .getByRole("article")
    .getByText(b.memory.content, { exact: true })
    .waitFor();
  await shot("isolated-project-b-memory");
  assert.deepEqual(await read(a, `runs/${a.runId}`), before);
  assert.deepEqual(await allSessions(a), sessionsBefore.A);
  assert.deepEqual(await allSessions(b), sessionsBefore.B);
  result.agentSessionCount = {
    A: sessionsBefore.A.length,
    B: sessionsBefore.B.length,
  };
  result.originalInvocationCount =
    sessionsBefore.A.length + sessionsBefore.B.length;
  result.tests.push({
    name: "Restoration and final browser steps create no new agents, attempts, reviews, integrations or model requests",
    status: "passed",
  });
  result.usageAfterDeletion = await read(
    a,
    `memory/usage?sessionId=${a.secondWorker.id}`,
  );
  assert.ok(
    result.usageAfterDeletion[0].memories.some(
      (m) => m.memoryId === a.memory.id && m.revision === 5,
    ),
  );
  const diff = spawnSync(
    "git",
    ["-C", a.root, "diff", "--binary", a.initialHead, a.integrationCommit],
    { encoding: "utf8", windowsHide: true },
  );
  assert.equal(diff.status, 0);
  await fs.writeFile(path.join(output, "implementation.patch"), diff.stdout);
  result.patchSha256 = hash(diff.stdout);
  assert.deepEqual(result.pageErrors, []);
  result.status = "PASS";
} catch (error) {
  result.status = "FAILED";
  result.error = String(error.stack ?? error).replaceAll(
    csrf ?? "__unset_csrf__",
    "[REDACTED]",
  );
  throw error;
} finally {
  await browser?.close();
  await inspector?.dispose();
  if (server.exitCode === null) {
    const exit = new Promise((r) => server.once("exit", r));
    server.kill();
    await exit;
  }
  result.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "results.json"),
    JSON.stringify(result, null, 2) + "\n",
  );
}
