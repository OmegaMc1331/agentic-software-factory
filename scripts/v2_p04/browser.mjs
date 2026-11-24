import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import os from "node:os";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const output = path.resolve(root, process.env.ASF_P04_OUTPUT_DIR ?? ".factory/v2/p04");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const binary =
  process.env.ASF_P04_BINARY ??
  path.join(
    root,
    "target/debug",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const fixture = await fs.mkdtemp(path.join(output, "browser-fixtures-"));
const a = path.join(fixture, "Fixture A"),
  b = path.join(fixture, "Fixture B");
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p04-application-")),
  "registry.sqlite3",
);
const results = {
  schema: 1,
  started: new Date().toISOString(),
  fixture,
  registry,
  binary,
  synthetic: true,
  tests: [],
  screenshots: [],
  startupMs: [],
  switchMs: [],
  requestMs: [],
  audits: [],
  errors: [],
  serverPids: [],
  memory: [],
  unexpectedRequests: [],
};
const run = (program, args, cwd) => {
  const value = spawnSync(program, args, {
    cwd,
    windowsHide: true,
    encoding: "utf8",
  });
  assert.equal(value.status, 0, value.stderr);
  return value.stdout;
};
const serverEnvironment = { ...process.env };
if (process.platform === "win32") {
  const git = run("where.exe", ["git"], root).trim().split(/\r?\n/)[0];
  const system = process.env.SystemRoot ?? "C:\\Windows";
  for (const name of Object.keys(serverEnvironment))
    if (name.toLowerCase() === "path") delete serverEnvironment[name];
  serverEnvironment.PATH = [
    path.dirname(git),
    path.join(system, "System32"),
    system,
  ].join(path.delimiter);
  results.serverPath = serverEnvironment.PATH;
  results.serverPathExcludesNodeAndRust = true;
}
function sampleMemory(label) {
  if (process.platform !== "win32") return;
  assert.ok(Number.isSafeInteger(child.pid));
  const data = JSON.parse(
    run(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Get-Process -Id ${child.pid} | Select-Object Id,PrivateMemorySize64,WorkingSet64 | ConvertTo-Json -Compress`,
      ],
      root,
    ),
  );
  results.memory.push({ label, ...data });
}
for (const [directory, label] of [
  [a, "A"],
  [b, "B"],
]) {
  await fs.mkdir(directory);
  run("git", ["init", "-q", "-b", "main"], directory);
  run(binary, ["init"], directory);
  await fs.writeFile(path.join(directory, ".gitignore"), ".factory/\n");
  await fs.writeFile(
    path.join(directory, "same.txt"),
    `controlled project ${label}\n`,
  );
  run(
    "git",
    [
      "-c",
      "user.name=P04 Fixture",
      "-c",
      "user.email=p04@example.test",
      "add",
      ".",
    ],
    directory,
  );
  run(
    "git",
    [
      "-c",
      "user.name=P04 Fixture",
      "-c",
      "user.email=p04@example.test",
      "commit",
      "-qm",
      "controlled fixture",
    ],
    directory,
  );
  const script =
    "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute(\"INSERT INTO runs(objective,status,created_at,updated_at) VALUES(?, 'planned','2026-10-27','2026-10-27')\",('Controlled '+sys.argv[2]+' workflow',));c.commit()";
  run(
    process.env.PYTHON ?? "python",
    ["-c", script, path.join(directory, ".factory/db.sqlite3"), label],
    root,
  );
  await fs.writeFile(
    path.join(directory, ".factory/config.toml"),
    `[agents.codex]\ncommand = "git"\nargs = ["--version"]\n[agents.codex.env]\nASF_PROJECT_PROOF = "${label}"\n[[role_assignments]]\nrole = "worker"\nagent = "codex"\npreferred = true\n`,
  );
}
const port = await new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
let child;
let browser;
let context;
let activePage;
async function start() {
  const time = performance.now();
  const log = await fs.open(path.join(output, "browser-server.log"), "a");
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
    {
      cwd: a,
      env: serverEnvironment,
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  results.serverPids.push(child.pid);
  await log.close();
  const deadline = Date.now() + 30000;
  while (true) {
    if (child.exitCode !== null)
      throw new Error(`server exited ${child.exitCode}`);
    try {
      const response = await fetch(`${base}/api/health`);
      if (response.ok) break;
    } catch {}
    assert.ok(Date.now() < deadline, "server startup deadline");
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  results.startupMs.push(performance.now() - time);
}
async function stop() {
  if (!child || child.exitCode !== null) return;
  await new Promise((resolve) => {
    child.once("exit", resolve);
    child.kill();
  });
}
async function test(name, work) {
  const time = performance.now();
  try {
    await work();
    results.tests.push({
      name,
      status: "passed",
      ms: performance.now() - time,
    });
  } catch (error) {
    results.tests.push({ name, status: "failed", message: error.message });
    throw error;
  } finally {
    await fs.writeFile(
      path.join(output, "browser-results.json"),
      JSON.stringify(results, null, 2),
    );
  }
}
async function session() {
  const response = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(response.ok());
  return (await response.json()).csrfToken;
}
async function mutation(url, method, data) {
  const csrf = await session();
  const response = await context.request.fetch(`${base}/api${url}`, {
    method,
    headers: { Origin: base, "X-Factory-CSRF": csrf },
    data,
  });
  assert.ok(response.ok(), await response.text());
  const text = await response.text();
  return text ? JSON.parse(text) : undefined;
}

async function eventually(work, message = "condition", milliseconds = 15000) {
  const deadline = Date.now() + milliseconds;
  let last;
  while (Date.now() < deadline) {
    try {
      const value = await work();
      if (value) return value;
    } catch (error) {
      last = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 70));
  }
  throw new Error(`${message}: ${last?.message ?? "deadline"}`);
}
async function read(url) {
  const response = await context.request.get(`${base}/api${url}`);
  assert.ok(response.ok(), await response.text());
  return response.json();
}
async function saved(page) {
  const id = await page.evaluate(() =>
    localStorage.getItem("asf.v2.workspace.last"),
  );
  assert.ok(id);
  return read(`/workspaces/${id}`);
}
async function idle(page) {
  await eventually(async () => {
    const value = await page.locator(".ws-status").innerText();
    return value.includes("Workspace saved");
  }, "workspace durable save");
}
async function viewMenu(page, name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
async function layoutMenu(page, name) {
  await page
    .getByRole("button", { name: "Layout actions", exact: true })
    .click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
async function shot(page, name) {
  await page.screenshot({ path: path.join(output, "screenshots", name) });
  results.screenshots.push(name);
}
async function selectProject(page, id) {
  await page.getByLabel("Open registered project").selectOption(id);
  await page.locator(`#project-${id}[aria-selected="true"]`).waitFor();
  await idle(page);
}
async function frames(page) {
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  );
}
const leaves = (node) =>
  node.type === "leaf" ? [node] : node.data.flatMap(leaves);
const activeSession = (value) =>
  value.document.sessions.find(
    (session) => session.id === value.document.activeSessionId,
  );
async function pinView(page, projectId, kind) {
  await viewMenu(page, "Open another project's view beside this");
  await page.getByLabel("Project for this view").selectOption(projectId);
  await page.getByLabel("Project view type").selectOption(kind);
  await page
    .getByRole("button", { name: "Open beside current", exact: true })
    .click();
}
const terminalSessions = [];
try {
  const program = path.join(
    fixture,
    process.platform === "win32" ? "controlled-agent.exe" : "controlled-agent",
  );
  run(
    "rustc",
    [
      "--edition=2021",
      "crates/factory-api/tests/support/project_agent.rs",
      "-o",
      program,
    ],
    root,
  );
  for (const [directory, label] of [
    [a, "A"],
    [b, "B"],
  ]) {
    let configuration = "";
    for (const [role, name, mode] of [
      ["planner", "planner", "planner"],
      ["worker", "codex", "worker"],
      ["reviewer", "reviewer", "reviewer"],
    ]) {
      configuration += `[agents.${name}]\nkind="custom"\ncommand=${JSON.stringify(program.replaceAll("\\", "/"))}\nargs=["${mode}"]\nmax_concurrency=4\ninteractive_args=["terminal"]\n[agents.${name}.env]\nASF_PROJECT_PROOF="${label}"\nASF_WORKER_GATE=${JSON.stringify(path.join(fixture, `go-${label}`).replaceAll("\\", "/"))}\n`;
    }
    for (const [role, name] of [
      ["planner", "planner"],
      ["worker", "codex"],
      ["reviewer", "reviewer"],
    ])
      configuration += `[[role_assignments]]\nrole="${role}"\nagent="${name}"\npreferred=true\n`;
    await fs.writeFile(
      path.join(directory, ".factory/config.toml"),
      configuration,
    );
    run("git", ["config", "user.name", "P04 controlled fixture"], directory);
    run("git", ["config", "user.email", "p04@example.test"], directory);
  }
  run(
    "python",
    [
      "-c",
      "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]);c.executemany(\"INSERT INTO runs(objective,status,created_at,updated_at) VALUES(?,'completed','2026-10-27','2026-10-27')\",[(f'PERFORMANCE FIXTURE workflow {i} with a long objective for repository navigation',) for i in range(2,201)]);c.commit()",
      path.join(a, ".factory/db.sqlite3"),
    ],
    root,
  );
  results.assetMode = binary.includes("development")
    ? "disk-development"
    : "embedded";
  await start();
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  results.browser = browser.version();
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    colorScheme: "dark",
    reducedMotion: "reduce",
  });
  await session();
  const page = await context.newPage();
  activePage = page;
  page.setDefaultTimeout(10000);
  page.on("pageerror", (error) => results.errors.push(error.message));
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== base)
      results.unexpectedRequests.push(request.url());
  });
  const registered = await read("/projects");
  results.aid = registered[0].project_id;
  const entry = await mutation("/projects/register", "POST", {
    path: b,
    display_name: "Project B",
    trust_project: true,
  });
  results.bid = entry.project_id;
  await mutation(`/projects/${entry.project_id}/open`, "POST");
  results.registeredProjects = [results.aid, results.bid];
  for (let i = 2; i < 8; i++) {
    const directory = path.join(
      fixture,
      `Project ${i} long descriptive repository name`,
    );
    await fs.mkdir(directory);
    run("git", ["init", "-q", "-b", "main"], directory);
    run(binary, ["init"], directory);
    const project = await mutation("/projects/register", "POST", {
      path: directory,
      display_name: `Project ${i} long descriptive repository name`,
      trust_project: true,
    });
    results.registeredProjects.push(project.project_id);
    if (i === 2) await mutation(`/projects/${project.project_id}/open`, "POST");
  }
  for (const id of [results.aid, results.bid]) {
    const terminal = await mutation(
      `/projects/${id}/agents/codex/sessions`,
      "POST",
      { cols: 100, rows: 28 },
    );
    terminalSessions.push({ projectId: id, id: terminal.id });
  }
  assert.equal(terminalSessions[0].id, terminalSessions[1].id);
  results.terminalSessions = terminalSessions;
  const traffic = [];
  page.on("request", (request) =>
    traffic.push({ method: request.method(), url: request.url() }),
  );
  const sockets = [];
  page.on("websocket", (socket) => {
    const value = { url: socket.url(), received: [], sent: [] };
    socket.on("framereceived", (event) =>
      value.received.push(
        Buffer.isBuffer(event.payload)
          ? event.payload.toString()
          : event.payload,
      ),
    );
    socket.on("framesent", (event) => value.sent.push(String(event.payload)));
    sockets.push(value);
  });
  await test("WS-01 production workspace and two real registered projects", async () => {
    await page.goto(`${base}/workspace.html`);
    await page
      .locator(`.ws-view:visible[data-project-id="${results.aid}"] .ws-view-context`)
      .getByText("Fixture A", { exact: true })
      .first()
      .waitFor();
    await idle(page);
    await page.getByLabel("Open registered project").selectOption(results.bid);
    await page
      .locator(`.ws-view:visible[data-project-id="${results.bid}"] .ws-view-context`)
      .getByText("Project B", { exact: true })
      .first()
      .waitFor();
    await idle(page);
    const current = await saved(page);
    assert.equal(current.document.openProjects.length, 2);
    assert.equal(current.document.activeProjectId, results.bid);
    assert.equal(current.document.sessions.length, 2);
    await shot(page, "default-dark.png");
  });
  await test("WS-02 identical run IDs retain real project ownership", async () => {
    await page
      .getByRole("button", { name: /#1 Controlled B workflow/ })
      .first()
      .click();
    await page
      .getByRole("heading", { name: "Controlled B workflow", exact: true })
      .first()
      .waitFor();
    await idle(page);
    assert.equal(
      activeSession(await saved(page)).views.find((v) => v.kind === "workflow")
        .projectId,
      results.bid,
    );
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await viewMenu(page, "Workflows");
    await page
      .getByRole("button", { name: "Fetch next 100", exact: true })
      .click();
    await page.getByLabel("Filter workflow page", { exact: true }).fill("Controlled A workflow");
    await page
      .getByRole("button", { name: /#1 Controlled A workflow/ })
      .first()
      .click();
    await page
      .getByRole("heading", { name: "Controlled A workflow", exact: true })
      .first()
      .waitFor();
    await idle(page);
    assert.equal(
      activeSession(await saved(page)).views.find((v) => v.kind === "workflow")
        .projectId,
      results.aid,
    );
    await shot(page, "two-projects.png");
  });
  let running;
  await test("WS-03 project switching preserves an actual running controlled worker", async () => {
    running = await mutation(`/projects/${results.aid}/runs`, "POST", {
      objective: "Controlled UI A execution",
    });
    await eventually(
      async () =>
        (await read(`/projects/${results.aid}/runs/${running.id}`)).run
          .status === "planned",
      "planner complete",
      30000,
    );
    await mutation(`/projects/${results.aid}/runs/${running.id}/start`, "POST");
    await eventually(
      async () => {
        const detail = await read(
          `/projects/${results.aid}/runs/${running.id}`,
        );
        return (
          detail.tasks.some((task) => task.state === "running") &&
          detail.sessions.some(
            (session) =>
              session.agent === "codex" &&
              ["running", "active"].includes(session.status),
          )
        );
      },
      "real active worker",
      20000,
    );
    await page.getByRole("tab", { name: "Project B", exact: true }).click();
    assert.equal(
      (await read(`/projects/${results.aid}/runs/${running.id}`)).run.status,
      "active",
    );
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await viewMenu(page, "Workflows");
    await page
      .getByRole("button", { name: "Newest workflows", exact: true })
      .click();
    await page.getByLabel("Filter workflow page", { exact: true }).fill("");
    await page
      .getByRole("button", { name: new RegExp(`#${running.id} `) })
      .first()
      .click();
    await page
      .getByRole("heading", { name: "controlled project A", exact: true })
      .first()
      .waitFor();
  });
  await test("WS-12 closing the active workflow tab does not cancel or suspend its process", async () => {
    await layoutMenu(page, "Close active view");
    assert.equal(
      (await read(`/projects/${results.aid}/runs/${running.id}`)).run.status,
      "active",
    );
    await fs.writeFile(path.join(fixture, "go-A"), "release");
    await eventually(
      async () =>
        (await read(`/projects/${results.aid}/runs/${running.id}`)).run
          .status === "completed",
      "worker reviewed and integrated",
      30000,
    );
    const detail = await read(`/projects/${results.aid}/runs/${running.id}`);
    assert.ok(detail.attempts.some((attempt) => attempt.commitSha));
    results.controlledWorkflow = {
      id: running.id,
      projectId: results.aid,
      status: detail.run.status,
      worktree: detail.tasks[0].worktreePath,
    };
  });
  await test("WS-04 delayed A responses cannot replace visible B data", async () => {
    await page.getByRole("tab", { name: "Project B", exact: true }).click();
    let release;
    let intercepted;
    const gate = new Promise((resolve) => {
      release = resolve;
    });
    const captured = new Promise((resolve) => {
      intercepted = resolve;
    });
    const pattern = `**/api/projects/${results.aid}/runs?limit=25`;
    await page.route(pattern, async (route) => {
      const response = await route.fetch();
      intercepted();
      await gate;
      await route.fulfill({ response }).catch(() => {});
    });
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await Promise.race([
      captured,
      new Promise((_, reject) =>
        setTimeout(
          () => reject(new Error("A response not intercepted")),
          10000,
        ),
      ),
    ]);
    await page.getByRole("tab", { name: "Project B", exact: true }).click();
    release();
    await page.unroute(pattern);
    await idle(page);
    await page
      .getByRole("heading", { name: "Controlled B workflow", exact: true })
      .first()
      .waitFor();
    assert.equal((await saved(page)).document.activeProjectId, results.bid);
  });
  let noteId;
  await test("WS-05 native tab drag and accessible reorder preserve draft identity", async () => {
    await page
      .locator(".ws-presets")
      .getByRole("button", { name: "Code", exact: true })
      .click();
    await viewMenu(page, "Workspace note");
    const note = page.locator('.cm-content[aria-label^="Workspace note"]');
    await note.fill(
      "Retained workspace draft - never written to the repository.",
    );
    await idle(page);
    let value = await saved(page);
    noteId = activeSession(value).views.findLast(
      (v) => v.kind === "scratch",
    ).id;
    await layoutMenu(page, "Split vertically");
    await idle(page);
    value = await saved(page);
    const code = activeSession(value).views.find((v) => v.kind === "code");
    await page
      .locator(`[data-workspace-tab="${noteId}"]`)
      .dragTo(page.locator(`[data-workspace-tab="${code.id}"]`), {
        targetPosition: { x: 45, y: 12 },
      });
    await idle(page);
    const grouped = activeSession(await saved(page));
    assert.ok(
      leaves(grouped.layout.grid.root).some(
        (leaf) =>
          leaf.data.views.includes(noteId) && leaf.data.views.includes(code.id),
      ),
    );
    await page.locator(`[data-workspace-tab="${noteId}"]`).click();
    await layoutMenu(page, "Move tab left");
    await idle(page);
    assert.equal(
      activeSession(await saved(page)).views.find((v) => v.id === noteId).state
        .draft,
      "Retained workspace draft - never written to the repository.",
    );
  });
  await test("WS-06 real cross-project views coexist in split groups", async () => {
    await pinView(page, results.aid, "agents");
    await page
      .getByRole("heading", { name: "Configured agents", exact: true })
      .waitFor();
    await idle(page);
    const current = activeSession(await saved(page));
    assert.ok(
      current.views.some(
        (v) => v.projectId === results.aid && v.kind === "agents",
      ),
    );
    assert.equal(current.projectId, results.bid);
    assert.ok(leaves(current.layout.grid.root).length >= 3);
    await shot(page, "split-workspace.png");
  });
  await test("WS-07 exact split tree tabs active project sizes draft and theme restore", async () => {
    await page.getByLabel("Theme", { exact: true }).selectOption("light");
    await idle(page);
    const before = await saved(page);
    await page.reload();
    await idle(page);
    const after = await saved(page);
    assert.equal(
      after.document.activeProjectId,
      before.document.activeProjectId,
    );
    assert.equal(
      after.document.activeSessionId,
      before.document.activeSessionId,
    );
    assert.deepEqual(activeSession(after).layout, activeSession(before).layout);
    assert.equal(
      activeSession(after).views.find((v) => v.id === noteId).state.draft,
      "Retained workspace draft - never written to the repository.",
    );
    assert.equal(
      await page.locator("html").getAttribute("data-asf-theme"),
      "light",
    );
    await shot(page, "restored-workspace.png");
  });
  await test("WS-13 all four presets preserve resources and unsaved drafts", async () => {
    for (const preset of ["Focus", "Code", "Agents", "Review"]) {
      await page
        .locator(".ws-presets")
        .getByRole("button", { name: preset, exact: true })
        .click();
      await idle(page);
      assert.equal(
        activeSession(await saved(page)).views.find((v) => v.id === noteId)
          .state.draft,
        "Retained workspace draft - never written to the repository.",
      );
      await shot(page, `${preset.toLowerCase()}-preset.png`);
    }
    await page.getByLabel("Theme", { exact: true }).selectOption("dark");
    await idle(page);
  });
  await test("WS-11 cross-project SSE and actual terminal sockets remain separately scoped", async () => {
    await page
      .locator(".ws-presets")
      .getByRole("button", { name: "Focus", exact: true })
      .click();
    await viewMenu(page, "Agents");
    let agent = page
      .locator(".ws-agent")
      .filter({
        has: page.getByRole("heading", { name: "codex", exact: true }),
      })
      .first();
    await agent
      .getByRole("button", { name: "Open output", exact: true })
      .first()
      .click();
    await page.getByLabel(`Recorded output ${results.bid} session 1`).waitFor();
    assert.ok(
      (
        await page
          .getByLabel(`Recorded output ${results.bid} session 1`)
          .getAttribute("data-working-directory")
      ).includes("Fixture B"),
    );
    await page
      .locator(`.ws-view[data-project-id="${results.bid}"]`)
      .filter({
        has: page.getByLabel(`Recorded output ${results.bid} session 1`),
      })
      .getByRole("button", { name: "Open terminal", exact: true })
      .click();
    await eventually(
      () =>
        sockets.some(
          (socket) =>
            socket.url.includes(results.bid) &&
            socket.received.join("").includes("terminal-B"),
        ),
      "B terminal output",
    );
    await pinView(page, results.aid, "agents");
    agent = page
      .locator(`.ws-view[data-project-id="${results.aid}"] .ws-agent`)
      .filter({
        has: page.getByRole("heading", { name: "codex", exact: true }),
      })
      .first();
    await agent
      .getByRole("button", { name: "Open output", exact: true })
      .filter({ hasText: "Open output" })
      .last()
      .click();
    await page.getByLabel(`Recorded output ${results.aid} session 1`).waitFor();
    assert.ok(
      (
        await page
          .getByLabel(`Recorded output ${results.aid} session 1`)
          .getAttribute("data-working-directory")
      ).includes("Fixture A"),
    );
    await page
      .locator(`.ws-view[data-project-id="${results.aid}"]`)
      .filter({
        has: page.getByLabel(`Recorded output ${results.aid} session 1`),
      })
      .getByRole("button", { name: "Open terminal", exact: true })
      .click();
    await page
      .getByRole("region", { name: `Agent terminal ${results.aid} session 1` })
      .waitFor();
    await eventually(
      () =>
        sockets.some(
          (socket) =>
            socket.url.includes(results.aid) &&
            socket.received.join("").includes("terminal-A"),
        ),
      "A terminal output",
    );
    const input = page
      .getByRole("region", { name: `Agent terminal ${results.aid} session 1` })
      .locator("textarea");
    await input.focus();
    await page.keyboard.type("workspace-proof\r");
    await eventually(
      () =>
        sockets.some(
          (socket) =>
            socket.url.includes(results.aid) &&
            socket.received.join("").includes("A:workspace-proof"),
        ),
      "A terminal input echo",
    );
    assert.ok(
      !sockets.some(
        (socket) =>
          socket.url.includes(results.bid) &&
          socket.received.join("").includes("A:workspace-proof"),
      ),
    );
    results.streamEvidence = {
      sse: traffic.filter((t) => t.url.includes("/stream")).map((t) => t.url),
      sockets,
    };
  });
  await test("WS-18 move hide restore and preset changes keep one active stream subscription", async () => {
    const before = traffic.filter((t) =>
      t.url.endsWith(`/sessions/1/stream`),
    ).length;
    await layoutMenu(page, "Move to next group");
    await layoutMenu(page, "Hide active view");
    await viewMenu(page, "Reopen Terminal #1");
    await page
      .locator(".ws-presets")
      .getByRole("button", { name: "Agents", exact: true })
      .click();
    await idle(page);
    assert.equal(
      traffic.filter((t) => t.url.endsWith(`/sessions/1/stream`)).length,
      before,
    );
    assert.equal(
      sockets.filter((socket) => socket.url.includes(results.aid)).length,
      1,
    );
    assert.ok(
      ["active", "running"].includes(
        (await read(`/projects/${results.aid}/sessions/1`)).status,
      ),
    );
  });
  for (const terminal of terminalSessions)
    await mutation(
      `/projects/${terminal.projectId}/sessions/${terminal.id}`,
      "DELETE",
    );
  await test("independent navigation sessions restore their own drafts and project tab closure is navigation only", async () => {
    await idle(page);
    const original = activeSession(await saved(page)).id;
    await page
      .getByRole("button", { name: "New workspace session", exact: true })
      .click();
    await viewMenu(page, "Workspace note");
    await page
      .locator('.cm-content[aria-label^="Workspace note"]:visible')
      .fill("Independent session draft");
    await idle(page);
    const second = activeSession(await saved(page)).id;
    assert.notEqual(second, original);
    await page
      .getByLabel("Workspace session", { exact: true })
      .selectOption(original);
    await idle(page);
    assert.equal(
      activeSession(await saved(page)).views.find((v) => v.id === noteId).state
        .draft,
      "Retained workspace draft - never written to the repository.",
    );
    await page
      .getByLabel("Workspace session", { exact: true })
      .selectOption(second);
    await idle(page);
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await page.getByRole("tab", { name: "Project B", exact: true }).click();
    await idle(page);
    assert.equal(activeSession(await saved(page)).id, second);
    await page
      .getByRole("button", { name: "Hide session", exact: true })
      .click();
    await idle(page);
    await page
      .getByLabel("Workspace session", { exact: true })
      .selectOption(second);
    await idle(page);
    assert.equal(
      activeSession(await saved(page)).views.find((v) => v.kind === "scratch")
        .state.draft,
      "Independent session draft",
    );
    await page.getByRole("tab", { name: "Project B", exact: true }).focus();
    await page.keyboard.press("Delete");
    await idle(page);
    assert.equal(
      (await read("/projects")).find((p) => p.project_id === results.bid)
        .lifecycle,
      "open",
    );
    await selectProject(page, results.bid);
    assert.equal(activeSession(await saved(page)).id, second);
    await page
      .getByLabel("Workspace session", { exact: true })
      .selectOption(original);
    await idle(page);
  });
  await test("WS-14 simultaneous windows detect revision conflict and fork safely", async () => {
    await idle(page);
    const original = (await saved(page)).document.id;
    const second = await context.newPage();
    await second.goto(`${base}/workspace.html`);
    await idle(second);
    await second
      .locator(".ws-presets")
      .getByRole("button", { name: "Review", exact: true })
      .click();
    await idle(second);
    await page
      .locator(".ws-presets")
      .getByRole("button", { name: "Code", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Save independent copy", exact: true })
      .waitFor();
    await page
      .getByRole("button", { name: "Save independent copy", exact: true })
      .click();
    await idle(page);
    assert.notEqual((await saved(page)).document.id, original);
    assert.equal(
      activeSession(await read(`/workspaces/${original}`)).preset,
      "Review",
    );
    await second.close();
  });
  await test("WS-16 keyboard group navigation dirty close confirmation and numeric resizing", async () => {
    await viewMenu(page, "Workspace note");
    await page
      .locator('.cm-content[aria-label^="Workspace note"]:visible')
      .fill("Keyboard protected draft");
    await page.keyboard.press("Alt+Shift+W");
    await page
      .getByRole("alertdialog", { name: "Close view with a retained draft?" })
      .waitFor();
    await shot(page, "dirty-close-confirmation.png");
    await page.keyboard.press("Escape");
    assert.equal(await page.getByRole("alertdialog").count(), 0);
    await page.keyboard.press("F6");
    await page.keyboard.press("Alt+Shift+P");
    assert.equal(
      await page
        .getByLabel("Open registered project")
        .evaluate((element) => document.activeElement === element),
      true,
    );
    await page.keyboard.press("Tab");
    await layoutMenu(page, "Resize active panel");
    await page.getByLabel("Width in pixels").fill("440");
    await page.getByRole("button", { name: "Apply size", exact: true }).click();
    await idle(page);
  });
  await test("WS-15 legacy dashboard retains scoped workflows graph consoles and settings", async () => {
    const legacy = await context.newPage();
    await legacy.goto(`${base}/?project=${results.bid}`);
    await legacy
      .getByRole("button", { name: "Agent Graph", exact: true })
      .waitFor();
    assert.ok(
      (await read(`/projects/${results.bid}/runs/1`)).run.objective.includes(
        "B",
      ),
    );
    for (const name of ["Workflows", "Settings"]) {
      const button = legacy.getByRole("button", { name, exact: true });
      if (await button.count()) await button.click();
    }
    assert.equal(
      (await legacy.locator("body").innerText()).includes(
        "Controlled A workflow",
      ),
      false,
    );
    await legacy.close();
  });
  await test("WS-17 production entry keeps authentication Origin and CSRF enforcement", async () => {
    const unauth = await browser.newContext();
    assert.equal(
      (await unauth.request.get(`${base}/api/workspaces`)).status(),
      401,
    );
    assert.equal(
      (
        await context.request.put(
          `${base}/api/workspaces/${(await saved(page)).document.id}`,
          { headers: { Origin: base }, data: {} },
        )
      ).status(),
      403,
    );
    assert.equal(
      (
        await context.request.get(`${base}/api/projects`, {
          headers: { Origin: "https://untrusted.example" },
        })
      ).status(),
      403,
    );
    await unauth.close();
  });
  await test("browser close and reopen restores a persistent profile without rebuilding its navigation", async () => {
    await idle(page);
    const copy = structuredClone((await saved(page)).document);
    copy.id = `w_${crypto.randomUUID().replaceAll("-", "")}`;
    await mutation(`/workspaces/${copy.id}`, "PUT", {
      base_revision: 0,
      document: copy,
    });
    const profile = path.join(fixture, "persistent-browser-profile");
    const options = {
      channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
      headless: true,
      viewport: { width: 1440, height: 900 },
      reducedMotion: "reduce",
    };
    let persistent;
    try {
      persistent = await chromium.launchPersistentContext(profile, options);
      await persistent.addCookies(await context.cookies());
      const first = persistent.pages()[0];
      await first.goto(`${base}/api/health`);
      await first.evaluate(
        (id) => localStorage.setItem("asf.v2.workspace.last", id),
        copy.id,
      );
      await first.goto(`${base}/workspace.html`);
      await idle(first);
      const before = await (
        await persistent.request.get(`${base}/api/workspaces/${copy.id}`)
      ).json();
      await persistent.close();
      persistent = await chromium.launchPersistentContext(profile, options);
      const reopened = persistent.pages()[0];
      await reopened.goto(`${base}/workspace.html`);
      await idle(reopened);
      assert.equal(
        await reopened.evaluate(() =>
          localStorage.getItem("asf.v2.workspace.last"),
        ),
        copy.id,
      );
      const after = await (
        await persistent.request.get(`${base}/api/workspaces/${copy.id}`)
      ).json();
      assert.equal(
        after.document.activeProjectId,
        before.document.activeProjectId,
      );
      assert.equal(
        after.document.activeSessionId,
        before.document.activeSessionId,
      );
      assert.deepEqual(
        activeSession(after).layout,
        activeSession(before).layout,
      );
      results.persistentBrowserReopen = true;
    } finally {
      await persistent?.close();
    }
  });
  await test("renamed project and removed resource preserve stable view identities with a visible error", async () => {
    await mutation(`/projects/${results.bid}`, "PUT", {
      display_name: "Project B renamed",
    });
    await page.reload();
    await idle(page);
    await page
      .getByRole("tab", { name: "Project B renamed", exact: true })
      .waitFor();
    const doc = structuredClone((await saved(page)).document),
      target = activeSession({ document: doc });
    const view = target.views.find(
      (v) => v.kind === "workflow" && v.projectId === results.bid,
    );
    assert.ok(view);
    const removed = structuredClone(view);
    removed.id = `v_${crypto.randomUUID().replaceAll("-", "")}`;
    removed.resourceId = 999999;
    removed.title = "Removed controlled workflow";
    removed.closed = false;
    removed.hidden = false;
    target.views.push(removed);
    doc.id = `w_${crypto.randomUUID().replaceAll("-", "")}`;
    const group = leaves(target.layout.grid.root).find((leaf) =>
      leaf.data.views.includes(view.id),
    );
    assert.ok(group);
    group.data.views.push(removed.id);
    group.data.activeView = removed.id;
    target.layout.activeGroup = group.data.id;
    target.activeViewId = removed.id;
    target.layout.panels[removed.id] = {
      ...target.layout.panels[view.id],
      id: removed.id,
      title: removed.title,
    };
    await mutation(`/workspaces/${doc.id}`, "PUT", {
      base_revision: 0,
      document: doc,
    });
    const isolated = await browser.newContext({
      storageState: await context.storageState(),
      viewport: { width: 1440, height: 900 },
      reducedMotion: "reduce",
    });
    try {
      const missing = await isolated.newPage();
      await missing.goto(`${base}/api/health`);
      await missing.evaluate(
        (id) => localStorage.setItem("asf.v2.workspace.last", id),
        doc.id,
      );
      await missing.goto(`${base}/workspace.html`);
      await missing
        .locator(`.ws-view[data-view-id="${removed.id}"]`)
        .getByRole("alert")
        .waitFor();
      assert.equal(
        (
          await (
            await isolated.request.get(`${base}/api/workspaces/${doc.id}`)
          ).json()
        ).document.sessions
          .find((s) => s.id === target.id)
          .views.find((v) => v.id === removed.id).projectId,
        results.bid,
      );
    } finally {
      await isolated.close();
    }
    await mutation(`/projects/${results.bid}`, "PUT", {
      display_name: "Project B",
    });
    await page.reload();
    await idle(page);
  });
  await test("WS-08 actual Rust server restart preserves workspace and safely reconnects projects", async () => {
    await idle(page);
    const before = await saved(page);
    await stop();
    await start();
    await session();
    await page.reload();
    await eventually(
      () =>
        page
          .locator(".ws-status")
          .innerText()
          .then((text) => text.includes("saved")),
      "restart workspace ready",
    );
    assert.equal((await saved(page)).document.id, before.document.id);
    const reopen = page.getByRole("button", {
      name: "Open project runtime",
      exact: true,
    });
    if (await reopen.count()) await reopen.first().click();
    await eventually(
      async () =>
        (await read("/projects")).find((p) => p.project_id === results.bid)
          .lifecycle === "open",
      "B explicitly reopened",
    );
    await page
      .getByRole("button", { name: "Reconnect / retry save", exact: true })
      .first()
      .click()
      .catch(() => {});
    await idle(page);
  });
  await test("WS-09 unavailable project retains navigation while healthy A remains accessible", async () => {
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await idle(page);
    await mutation(`/projects/${results.bid}/close`, "POST");
    await fs.rename(b, path.join(fixture, "Moved B"));
    await page.getByRole("tab", { name: "Project B", exact: true }).click();
    await idle(page);
    await page.reload();
    await page
      .locator(".ws-view")
      .getByRole("alert")
      .filter({ hasText: /specified|introuvable|unavailable/i })
      .first()
      .waitFor();
    await shot(page, "project-unavailable.png");
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await page
      .locator(".ws-presets")
      .getByRole("button", { name: "Focus", exact: true })
      .click();
    await page
      .locator(`.ws-view:visible[data-project-id="${results.aid}"] .ws-view-context`)
      .getByText("Fixture A", { exact: true })
      .first()
      .waitFor();
    await idle(page);
  });
  await test("WS-10 malformed saved layout recovers safely without replacing the stored document", async () => {
    const current = await saved(page),
      bad = { ...current.document, version: 99 };
    run(
      "python",
      [
        "-c",
        "import sqlite3,sys;c=sqlite3.connect(sys.argv[1]);c.execute('UPDATE workspaces SET document=? WHERE id=?',(sys.argv[2],sys.argv[3]));c.commit()",
        registry.replace(/\.sqlite3$/, ".workspaces.sqlite3"),
        JSON.stringify(bad),
        current.document.id,
      ],
      root,
    );
    await page.reload();
    await page
      .getByText(/invalid or unsupported schema/)
      .first()
      .waitFor();
    await page
      .getByRole("button", { name: "Open safe empty workspace", exact: true })
      .click();
    await page
      .locator(`.ws-view:visible[data-project-id="${results.aid}"] .ws-view-context`)
      .getByText("Fixture A", { exact: true })
      .first()
      .waitFor();
    await idle(page);
    assert.equal(
      (await read(`/workspaces/${current.document.id}`)).document.version,
      99,
    );
    results.corruptLayoutRecovered = true;
  });
  await test("visual desktop themes responsive widths reduced motion and accessibility", async () => {
    for (const theme of ["dark", "light"]) {
      await page.getByLabel("Theme", { exact: true }).selectOption(theme);
      await idle(page);
      await shot(page, `default-${theme}.png`);
      await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
      const audit = await page.evaluate(async () => {
        const result = await window.axe.run(document, {
          runOnly: {
            type: "tag",
            values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
          },
        });
        return {
          violations: result.violations,
          incomplete: result.incomplete,
        };
      });
      results.audits.push({ theme, ...audit });
      const contrastSupplement = await page.evaluate((rules) => {
        const parse = (color) => {
          if (color.startsWith("#")) {
            const value = color.slice(1);
            return value.length === 6
              ? [0, 2, 4].map((i) => parseInt(value.slice(i, i + 2), 16))
              : null;
          }
          const values = color
            .match(/[\d.]+/g)
            ?.slice(0, 3)
            .map(Number);
          return values?.length === 3 ? values : null;
        };
        const luminance = (rgb) =>
          rgb
            .map((v) => {
              const x = v / 255;
              return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
            })
            .reduce((n, v, i) => n + v * [0.2126, 0.7152, 0.0722][i], 0);
        const style = getComputedStyle(document.documentElement);
        const surfaces = [
          "--color-panel",
          "--color-panel-muted",
          "--color-canvas",
        ].map((name) => ({
          name,
          rgb: parse(style.getPropertyValue(name).trim()),
        }));
        return rules
          .filter((r) => r.id === "color-contrast")
          .flatMap((r) => r.nodes)
          .map((node) => {
            const element = document.querySelector(node.target[0]);
            if (!element) throw new Error("Contrast target is missing.");
            const foreground = getComputedStyle(element).color,
              rgb = parse(foreground);
            if (!rgb || surfaces.some((s) => !s.rgb))
              throw new Error("Unsupported computed contrast color.");
            const ratios = surfaces.map((s) => {
              const x = luminance(rgb),
                y = luminance(s.rgb);
              return {
                surface: s.name,
                ratio: (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05),
              };
            });
            return {
              target: node.target,
              foreground,
              ratios,
              minimum: Math.min(...ratios.map((r) => r.ratio)),
            };
          });
      }, audit.incomplete);
      results.audits.at(-1).contrastSupplement = contrastSupplement;
      assert.ok(
        contrastSupplement.every((item) => item.minimum >= 4.5),
        JSON.stringify(contrastSupplement),
      );
      assert.equal(
        audit.violations.length,
        0,
        JSON.stringify(audit.violations),
      );
    }
    for (const [width, height] of [
      [1920, 1080],
      [1440, 900],
      [1280, 800],
      [1024, 768],
    ]) {
      await page.setViewportSize({ width, height });
      await idle(page);
      assert.ok(
        await page
          .getByRole("button", { name: "Views", exact: true })
          .isVisible(),
      );
      assert.ok(
        await page
          .getByRole("button", { name: "Layout actions", exact: true })
          .isVisible(),
      );
      await layoutMenu(page, "Resize active panel");
      await page
        .getByRole("dialog", { name: "Resize active panel", exact: true })
        .waitFor();
      await shot(page, `resize-dialog-${width}.png`);
      await page.keyboard.press("Escape");
      assert.ok(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      );
        assert.equal(
          await page.locator(".ws-dock").evaluate((element) => element.scrollTop),
          0,
          "The dock host cannot scroll panels outside the visible workspace",
        );
      if (width === 1024) await shot(page, "narrow-desktop.png");
    }
    await page.setViewportSize({ width: 1440, height: 900 });
  });
  await test("performance fixture eight registered three open thirty tabs and two hundred summaries", async () => {
    await mutation(`/projects/${results.registeredProjects[3]}/open`, "POST");
    await selectProject(page, results.registeredProjects[2]);
    await selectProject(page, results.registeredProjects[3]);
    await page.getByRole("tab", { name: "Fixture A", exact: true }).click();
    await idle(page);
    const current = await saved(page),
      doc = structuredClone(current.document),
      session = activeSession(current),
      target = doc.sessions.find((s) => s.id === session.id);
    const template = target.views.find((v) => v.kind === "workflows");
    for (let i = 0; i < 30; i++) {
      const id = `v_${(i + 1).toString(16).padStart(32, "0")}`;
      target.views.push({
        ...template,
        id,
        title: `PERFORMANCE TAB ${i} src/a/very/long/path/to/a/component/workspace-navigation.tsx`,
        closed: false,
        hidden: false,
        state: {},
      });
      target.layout.panels[id] = {
        ...target.layout.panels[template.id],
        id,
        title: `PERFORMANCE TAB ${i}`,
        params: {},
      };
      leaves(target.layout.grid.root)
        .find((leaf) => leaf.data.views.includes(template.id))
        .data.views.push(id);
    }
    await mutation(`/workspaces/${doc.id}`, "PUT", {
      base_revision: current.revision,
      document: doc,
    });
    const startTime = performance.now();
    await page.reload();
    await idle(page);
    results.performance = {
      restoreThirtyTabsMs: performance.now() - startTime,
      registered: (await read("/projects")).length,
      workflowSummaries: (await read(`/projects/${results.aid}/runs?limit=100`))
        .items.length,
      heap: await page.evaluate(() =>
        performance.memory
          ? {
              used: performance.memory.usedJSHeapSize,
              total: performance.memory.totalJSHeapSize,
            }
          : null,
      ),
      samples: [],
    };
    for (const preset of ["Focus", "Code", "Agents", "Review"]) {
      const time = performance.now();
      await page
        .locator(".ws-presets")
        .getByRole("button", { name: preset, exact: true })
        .click();
      await frames(page);
      results.performance.samples.push({
        preset,
        ms: performance.now() - time,
      });
    }
    await idle(page);
    await shot(page, "many-tabs-long-content.png");
    assert.ok(activeSession(await saved(page)).views.length >= 30);
    results.trafficCount = traffic.length;
    results.performance.projectSwitchSamples = [];
    for (let i = 0; i < 6; i++) {
      for (const id of [results.registeredProjects[2], results.aid]) {
        const time = performance.now();
        await page.locator(`#project-${id}`).click();
        await frames(page);
        results.performance.projectSwitchSamples.push({
          projectId: id,
          ms: performance.now() - time,
        });
      }
    }
    await idle(page);
  });
  await test("repeatable browser startup theme tab movement pointer resizing and cache measurements", async () => {
    const metrics = results.performance;
    metrics.openProjects = (await read("/projects")).filter(
      (p) => p.lifecycle === "open",
    ).length;
    assert.ok(metrics.openProjects >= 3);
    const counts = [];
    let cursor;
    do {
      const page = await read(
        `/projects/${results.aid}/runs?limit=100${cursor ? `&before=${cursor}` : ""}`,
      );
      counts.push(page.items.length);
      cursor = page.nextCursor;
    } while (cursor);
    metrics.workflowPageCounts = counts;
    assert.ok(counts.reduce((a, b) => a + b, 0) >= 200);
    metrics.theme = [];
    for (const theme of ["light", "dark", "light", "dark"]) {
      const time = performance.now();
      await page.getByLabel("Theme", { exact: true }).selectOption(theme);
      await frames(page);
      metrics.theme.push({ theme, ms: performance.now() - time });
    }
    metrics.openTabs = [];
    for (let i = 0; i < 5; i++) {
      const time = performance.now();
      await viewMenu(page, "Workspace note");
      await page
        .locator('.cm-content[aria-label^="Workspace note"]:visible')
        .waitFor();
      await frames(page);
      metrics.openTabs.push(performance.now() - time);
    }
    await idle(page);
    await shot(page, "code-note.png");
    metrics.moveTabs = [];
    const beforeReads = traffic.filter(
      (t) => t.method === "GET" && t.url.includes("/api/projects/"),
    ).length;
    for (let i = 0; i < 5; i++) {
      const time = performance.now();
      await layoutMenu(page, "Move to next group");
      await frames(page);
      metrics.moveTabs.push(performance.now() - time);
    }
    metrics.readRequestsDuringMoves =
      traffic.filter(
        (t) => t.method === "GET" && t.url.includes("/api/projects/"),
      ).length - beforeReads;
    metrics.resize = [];
    for (let i = 0; i < 3; i++) {
      const box = await page
        .locator(".dv-sash.dv-enabled")
        .first()
        .boundingBox();
      assert.ok(box);
      const vertical = box.height > box.width;
      const x = box.x + box.width / 2,
        y = box.y + box.height / 2;
      const time = performance.now();
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x + (vertical ? 24 : 0), y + (vertical ? 0 : 24), {
        steps: 4,
      });
      await page.mouse.up();
      await frames(page);
      metrics.resize.push(performance.now() - time);
    }
    await idle(page);
    const stable = await saved(page),
      bench = structuredClone(stable.document);
    bench.id = `w_${crypto.randomUUID().replaceAll("-", "")}`;
    await mutation(`/workspaces/${bench.id}`, "PUT", {
      base_revision: 0,
      document: bench,
    });
    metrics.cold = [];
    metrics.warm = [];
    for (let i = 0; i < 5; i++) {
      const storage = await context.storageState();
      storage.origins
        .find((origin) => origin.origin === base)
        .localStorage.find(
          (item) => item.name === "asf.v2.workspace.last",
        ).value = bench.id;
      const isolated = await browser.newContext({
        storageState: storage,
        viewport: { width: 1440, height: 900 },
        colorScheme: "dark",
        reducedMotion: "reduce",
      });
      const bp = await isolated.newPage();
      bp.on("pageerror", (error) => results.errors.push(error.message));
      await bp.goto(`${base}/workspace.html`);
      await bp.waitForFunction(
        () =>
          performance.getEntriesByName("asf-p04-workspace-ready").length > 0,
      );
      metrics.cold.push(
        await bp.evaluate(
          () =>
            performance.getEntriesByName("asf-p04-workspace-ready").at(-1)
              .startTime,
        ),
      );
      await idle(bp);
      await bp.reload();
      await bp.waitForFunction(
        () =>
          performance.getEntriesByName("asf-p04-workspace-ready").length > 0,
      );
      metrics.warm.push(
        await bp.evaluate(
          () =>
            performance.getEntriesByName("asf-p04-workspace-ready").at(-1)
              .startTime,
        ),
      );
      await idle(bp);
      await isolated.close();
    }
    metrics.heapAfter = await page.evaluate(() =>
      performance.memory
        ? {
            used: performance.memory.usedJSHeapSize,
            total: performance.memory.totalJSHeapSize,
          }
        : null,
    );
    sampleMemory("8 registered / 3+ open runtimes / 30+ views");
  });
  await test("empty application registry opens a real minimal workspace", async () => {
    const emptyRegistry = path.join(
      await fs.mkdtemp(path.join(os.tmpdir(), "asf-p04-empty-")),
      "registry.sqlite3",
    );
    const emptyPort = await new Promise((resolve) => {
      const server = net.createServer();
      server.listen(0, "127.0.0.1", () => {
        const value = server.address().port;
        server.close(() => resolve(value));
      });
    });
    const origin = `http://127.0.0.1:${emptyPort}`;
    const log = await fs.open(path.join(output, "empty-server.log"), "w");
    const server = spawn(
      binary,
      [
        "start",
        "--no-browser",
        "--no-default-project",
        "--registry",
        emptyRegistry,
        "--allow-project-root",
        fixture,
        "--port",
        String(emptyPort),
      ],
      {
        cwd: fixture,
        env: serverEnvironment,
        windowsHide: true,
        stdio: ["ignore", log.fd, log.fd],
      },
    );
    await log.close();
    let isolated;
    try {
      await eventually(async () => {
        if (server.exitCode !== null)
          throw new Error(`empty server exited ${server.exitCode}`);
        return (await fetch(`${origin}/api/health`)).ok;
      }, "empty registry server");
      isolated = await browser.newContext({
        viewport: { width: 1440, height: 900 },
        reducedMotion: "reduce",
      });
      const empty = await isolated.newPage();
      await empty.goto(`${origin}/workspace.html`);
      await empty
        .getByRole("heading", { name: "Open a project", exact: true })
        .waitFor();
      await eventually(
        async () =>
          (await isolated.request.get(`${origin}/api/projects`)).status() ===
          200,
        "empty application authenticated",
      );
      assert.deepEqual(
        await (await isolated.request.get(`${origin}/api/projects`)).json(),
        [],
      );
      await shot(empty, "empty-workspace.png");
    } finally {
      await isolated?.close();
      if (server.exitCode === null)
        await new Promise((resolve) => {
          server.once("exit", resolve);
          server.kill();
        });
    }
  });
  assert.deepEqual(results.errors, []);
  assert.deepEqual(results.unexpectedRequests, []);
} catch (error) {
  results.failure = error.stack;
  throw error;
} finally {
  if (activePage && results.failure)
    await activePage
      .screenshot({ path: path.join(output, "failure.png") })
      .catch(() => {});
  for (const label of ["A", "B"])
    await fs
      .writeFile(path.join(fixture, `go-${label}`), "cleanup")
      .catch(() => {});
  if (child?.exitCode === null && context)
    for (const terminal of terminalSessions)
      await mutation(
        `/projects/${terminal.projectId}/sessions/${terminal.id}`,
        "DELETE",
      ).catch(() => {});
  if (results.failure) {
    const attempt = path.join(output, "attempts", String(Date.now()));
    await fs.mkdir(attempt, { recursive: true });
    await fs.writeFile(
      path.join(attempt, "browser-results.json"),
      JSON.stringify(results, null, 2),
    );
    await fs
      .copyFile(
        path.join(output, "failure.png"),
        path.join(attempt, "failure.png"),
      )
      .catch(() => {});
  }
  await context?.close();
  await browser?.close();
  await stop();
  await fs.writeFile(
    path.join(output, "browser-results.json"),
    JSON.stringify(results, null, 2) + "\n",
  );
  console.log(
    JSON.stringify({
      tests: results.tests.length,
      passed: results.tests.filter((test) => test.status === "passed").length,
      errors: results.errors,
      failure: results.failure,
    }),
  );
}
