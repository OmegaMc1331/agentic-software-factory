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
const output = path.resolve(root, process.env.ASF_P03_OUTPUT_DIR ?? ".factory/v2/p03");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const binary =
  process.env.ASF_P03_BINARY ??
  path.join(
    root,
    "target/debug",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const fixture = await fs.mkdtemp(path.join(output, "browser-fixtures-"));
const a = path.join(fixture, "Fixture A"),
  b = path.join(fixture, "Fixture B");
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p03-application-")),
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
      "user.name=P03 Fixture",
      "-c",
      "user.email=p03@example.test",
      "add",
      ".",
    ],
    directory,
  );
  run(
    "git",
    [
      "-c",
      "user.name=P03 Fixture",
      "-c",
      "user.email=p03@example.test",
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
  return response.status() === 204 ? undefined : response.json();
}
try {
  await start();
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  results.browser = browser.version();
  context = await browser.newContext({
    viewport: { width: 1440, height: 960 },
    colorScheme: "light",
    reducedMotion: "reduce",
  });
  await session();
  const page = await context.newPage();
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== base)
      results.unexpectedRequests.push(request.url());
  });
  activePage = page;
  page.setDefaultTimeout(10000);
  page.on("pageerror", (error) => results.errors.push(error.message));
  const row = (name) =>
    page
      .locator("article.projects-row")
      .filter({ has: page.getByRole("button", { name, exact: true }) });
  const form = () => page.getByRole("dialog");
  await test("embedded V1 and projects assets use one authenticated local server", async () => {
    await page.goto(`${base}/`);
    await page.getByText("Controlled A workflow", { exact: true }).waitFor();
    await page.getByRole("link", { name: "Projects", exact: true }).click();
    await page.getByRole("heading", { name: "Local projects" }).waitFor();
    await page.getByText("Controlled A workflow", { exact: true }).waitFor();
    const api = await context.request.get(`${base}/api/projects`);
    const values = await api.json();
    results.aid = values[0].project_id;
    sampleMemory("one open project after initial authenticated UI reads");
  });
  await test("register and open B using validated backend path entry", async () => {
    await page.getByRole("button", { name: "Register", exact: true }).click();
    await form().getByLabel("Repository directory").fill(b);
    await form().getByRole("checkbox").check();
    await form()
      .getByRole("button", { name: "Register repository", exact: true })
      .click();
    await form().waitFor({ state: "hidden" });
    await row("Fixture B")
      .getByRole("button", { name: "Open", exact: true })
      .click();
    await page.getByText("Controlled B workflow", { exact: true }).waitFor();
    const values = await (
      await context.request.get(`${base}/api/projects`)
    ).json();
    results.bid = values.find((value) =>
      value.canonical_root.endsWith("Fixture B"),
    ).project_id;
    assert.notEqual(results.aid, results.bid);
    sampleMemory("two open projects after authenticated UI reads");
  });
  await test("same run IDs, agent names and filenames remain distinct", async () => {
    for (const [id, label] of [
      [results.aid, "A"],
      [results.bid, "B"],
    ]) {
      const value = await (
        await context.request.get(`${base}/api/projects/${id}/runs/1`)
      ).json();
      assert.equal(value.run.objective, `Controlled ${label} workflow`);
      const config = await (
        await context.request.get(`${base}/api/projects/${id}/config`)
      ).json();
      assert.equal(config.agents.codex.env.ASF_PROJECT_PROOF, label);
      const search = await (
        await context.request.get(
          `${base}/api/projects/${id}/context/search?q=controlled`,
        )
      ).json();
      assert.ok(
        search.hits.some(
          (hit) =>
            hit.path === "same.txt" && hit.snippet.includes(`project ${label}`),
        ),
      );
    }
  });
  await test("late A request cannot replace B, reload keeps B selected", async () => {
    let release;
    const gate = new Promise((resolve) => (release = resolve));
    let seen;
    const started = new Promise((resolve) => (seen = resolve));
    await page.route(
      (url) => url.pathname === `/api/projects/${results.aid}/runs`,
      async (route) => {
        const response = await route.fetch();
        seen();
        await gate;
        await route.fulfill({ response }).catch(() => {});
      },
    );
    await row("Fixture A")
      .getByRole("button", { name: "Select", exact: true })
      .click();
    await started;
    await row("Fixture B")
      .getByRole("button", { name: "Select", exact: true })
      .click();
    await page.getByText("Controlled B workflow", { exact: true }).waitFor();
    release();
    await page.unrouteAll({ behavior: "wait" });
    assert.equal(
      await page.getByText("Controlled A workflow", { exact: true }).count(),
      0,
    );
    await page.reload();
    await page.getByText("Controlled B workflow", { exact: true }).waitFor();
  });
  await test("project selection and scoped API latency samples", async () => {
    for (let i = 0; i < 12; i++) {
      const label = i % 2 ? "B" : "A";
      const time = performance.now();
      await row(`Fixture ${label}`)
        .getByRole("button", { name: "Select", exact: true })
        .click();
      await page
        .getByText(`Controlled ${label} workflow`, { exact: true })
        .waitFor();
      results.switchMs.push(performance.now() - time);
    }
    for (let i = 0; i < 20; i++) {
      const time = performance.now();
      const response = await context.request.get(
        `${base}/api/projects/${i % 2 ? results.aid : results.bid}/runs/1`,
      );
      assert.ok(response.ok());
      results.requestMs.push(performance.now() - time);
    }
  });
  await test("themes narrow desktop and accessibility", async () => {
    for (const theme of ["light", "dark"]) {
      await page.getByLabel("Theme").selectOption(theme);
      await page.evaluate(() => document.fonts.ready);
      await page.waitForFunction(
        () => !document.documentElement.dataset.themeChanging,
      );
      await page.screenshot({
        path: path.join(output, `screenshots/projects-${theme}.png`),
      });
      results.screenshots.push(`projects-${theme}.png`);
      await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
      const audit = await page.evaluate(async () =>
        window.axe.run(document, {
          runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag22aa"] },
        }),
      );
      results.audits.push({
        theme,
        violations: audit.violations.map(
          ({ id, impact, description, nodes }) => ({
            id,
            impact,
            description,
            targets: nodes.map((node) => node.target),
          }),
        ),
      });
      assert.equal(audit.violations.length, 0);
    }
    await page.setViewportSize({ width: 1024, height: 900 });
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    );
    await page.screenshot({
      path: path.join(output, "screenshots/projects-narrow.png"),
    });
    results.screenshots.push("projects-narrow.png");
    await page.setViewportSize({ width: 1440, height: 960 });
  });
  await test("create and clone actual projects without external network", async () => {
    await page
      .getByRole("button", { name: "Create project", exact: true })
      .click();
    await form()
      .getByLabel("New destination directory")
      .fill(path.join(fixture, "Created"));
    await form().getByLabel("Display name").fill("Created");
    await form().getByRole("checkbox").check();
    await form()
      .getByRole("button", { name: "Create project", exact: true })
      .click();
    await form().waitFor({ state: "hidden" });
    await row("Created").waitFor();
    await page.getByRole("button", { name: "Clone", exact: true }).click();
    await form()
      .getByLabel("New destination directory")
      .fill(path.join(fixture, "Cloned"));
    await form().getByLabel("Display name").fill("Cloned");
    await form().getByLabel("Git remote").fill(a);
    await form()
      .getByRole("checkbox", {
        name: "I trust this repository and its configured local tools",
      })
      .check();
    await form()
      .getByRole("button", { name: "Clone repository", exact: true })
      .click();
    await form().waitFor({ state: "hidden" });
    await row("Cloned").waitFor();
    assert.ok(
      (
        await fs.stat(path.join(fixture, "Cloned/.factory/db.sqlite3"))
      ).isFile(),
    );
  });
  await test("unavailable B is retained and explicitly relinked with stable ID", async () => {
    await row("Fixture B")
      .getByRole("button", { name: "Close when idle" })
      .click();
    await row("Fixture B")
      .getByRole("button", { name: "Open", exact: true })
      .waitFor();
    const moved = path.join(fixture, "Moved B");
    await fs.rename(b, moved);
    await page.getByRole("button", { name: "Refresh", exact: true }).click();
    await row("Fixture B").locator(".projects-problem").waitFor();
    await page.screenshot({
      path: path.join(output, "screenshots/projects-unavailable.png"),
    });
    results.screenshots.push("projects-unavailable.png");
    await row("Fixture B")
      .getByRole("button", { name: "Relink", exact: true })
      .click();
    await form().getByLabel("Repository directory").fill(moved);
    await form()
      .getByRole("checkbox", {
        name: "I trust this repository and its configured local tools",
      })
      .check();
    await form()
      .getByRole("button", { name: "Relink project", exact: true })
      .click();
    await form().waitFor({ state: "hidden" });
    await row("Fixture B")
      .getByRole("button", { name: "Open", exact: true })
      .click();
    await page.getByText("Controlled B workflow", { exact: true }).waitFor();
    const metadata = await (
      await context.request.get(`${base}/api/projects/${results.bid}`)
    ).json();
    assert.ok(metadata.canonical_root.endsWith("Moved B"));
  });
  await test("unauthorized destination fails visibly and retains entered data", async () => {
    await page
      .getByRole("button", { name: "Create project", exact: true })
      .click();
    await form()
      .getByLabel("New destination directory")
      .fill(path.join(root, "outside-p03-authorization"));
    await form().getByLabel("Display name").fill("Refused");
    await form().getByRole("checkbox").check();
    await form()
      .getByRole("button", { name: "Create project", exact: true })
      .click();
    await form().getByRole("alert").waitFor();
    assert.equal(
      await form().getByLabel("Display name").inputValue(),
      "Refused",
    );
    await page.screenshot({
      path: path.join(output, "screenshots/projects-refusal.png"),
    });
    results.screenshots.push("projects-refusal.png");
    await form().getByRole("button", { name: "Cancel", exact: true }).click();
  });
  await test("server restart restores registry IDs and independent stored run histories", async () => {
    const before = await (
      await context.request.get(`${base}/api/projects`)
    ).json();
    await stop();
    await start();
    await session();
    const after = await (
      await context.request.get(`${base}/api/projects`)
    ).json();
    assert.deepEqual(
      after.map((p) => p.project_id),
      before.map((p) => p.project_id),
    );
    await mutation(`/projects/${results.bid}/open`, "POST", {});
    for (const [id, label] of [
      [results.aid, "A"],
      [results.bid, "B"],
    ]) {
      const result = await (
        await context.request.get(`${base}/api/projects/${id}/runs/1`)
      ).json();
      assert.equal(result.run.objective, `Controlled ${label} workflow`);
    }
    await page.reload();
    await page.getByText("Controlled B workflow", { exact: true }).waitFor();
  });
  assert.equal(results.errors.length, 0);
  assert.equal(results.unexpectedRequests.length, 0);
} catch (error) {
  await activePage
    ?.screenshot({ path: path.join(output, "screenshots/failure.png") })
    .catch(() => {});
  results.failure = { message: error.message, stack: error.stack };
  throw error;
} finally {
  await fs.writeFile(
    path.join(output, "browser-results.json"),
    JSON.stringify(results, null, 2),
  );
  await context?.close();
  await browser?.close();
  await stop();
}
console.log(
  JSON.stringify({
    tests: results.tests.length,
    passed: results.tests.filter((test) => test.status === "passed").length,
    screenshots: results.screenshots.length,
  }),
);
