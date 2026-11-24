import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const output = path.join(root, ".factory/v2/p08/replay");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const acceptance = JSON.parse(await fs.readFile(path.join(root, ".factory/v2/p08/acceptance-parallel-r2/results.json"), "utf8"));
const binary = process.env.ASF_P08_BINARY ?? path.join(root, ".factory/v2/p08/run/factory-p08.exe");
const hash = data => createHash("sha256").update(data).digest("hex");
const result = { schema: 1, kind: "Durable replay of real accepted and refused missions; no new model invocation", started: new Date().toISOString(), sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8", windowsHide: true }).stdout.trim(), binarySha256: hash(await fs.readFile(binary)), modelBinarySha256: acceptance.binarySha256, tests: [], screenshots: [], assets: [], errors: [] };
try { await fs.copyFile(path.join(output, "results.json"), path.join(output, `results-${Date.now()}.json`)); } catch (error) { if (error.code !== "ENOENT") throw error; }
const port = await new Promise(resolve => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const value = s.address().port; s.close(() => resolve(value)); }); });
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(path.join(output, "server.log"), "w");
const child = spawn(binary, ["start", "--no-browser", "--port", String(port), "--registry", path.join(acceptance.fixture, "registry.sqlite3"), "--allow-project-root", acceptance.fixture], { cwd: acceptance.scenarios.Parallel.root, windowsHide: true, stdio: ["ignore", log.fd, log.fd] });
await log.close();
let browser, context, page;
async function until(fn, label) { for (let i = 0; i < 150; i++) { if (await fn()) return; await new Promise(resolve => setTimeout(resolve, 200)); } throw new Error(`Timeout: ${label}`); }
async function test(name, fn) { const start = performance.now(); try { await fn(); result.tests.push({ name, status: "passed", ms: performance.now() - start }); console.log(`PASS replay: ${name}`); } catch (error) { result.tests.push({ name, status: "failed", error: String(error) }); throw error; } }
async function get(project, suffix) { const response = await context.request.get(`${base}/api/projects/${project}/${suffix}`); assert.ok(response.ok(), await response.text()); return response.json(); }
async function shot(name) { await page.screenshot({ path: path.join(output, "screenshots", name + ".png"), fullPage: true }); result.screenshots.push(name); }
async function view(name) { await page.getByRole("button", { name: "Views", exact: true }).click(); await page.getByRole("menuitem", { name, exact: true }).click(); }
async function select(item) { await page.getByLabel("Open registered project").selectOption(item.project); await page.locator(`#project-${item.project}[aria-selected=true]`).waitFor(); }
try {
  await until(async () => { try { return (await fetch(base + "/api/health")).ok; } catch { return false; } }, "backend startup");
  const { chromium } = createRequire(path.join(root, "apps/dashboard/package.json"))("playwright");
  browser = await chromium.launch({ channel: "msedge", headless: true });
  context = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
  const session = await context.request.post(base + "/api/auth/session", { headers: { Origin: base }, data: {} });
  assert.ok(session.ok());
  const csrf = (await session.json()).csrfToken;
  for (const item of Object.values(acceptance.scenarios)) {
    const response = await context.request.post(`${base}/api/projects/${item.project}/open`, { headers: { Origin: base, "X-Factory-CSRF": csrf }, data: {} });
    assert.ok(response.ok(), await response.text());
  }
  page = await context.newPage();
  page.on("pageerror", error => result.errors.push(String(error)));
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").waitFor();
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  for (const name of ["Parallel", "Conflict"]) await test(`${name}: actual integration and review survive backend restart`, async () => {
    const item = acceptance.scenarios[name];
    const missions = await get(item.project, `conversations/${item.conversation}/missions`);
    const mission = missions.find(mission => mission.run_id === item.mission.run_id);
    assert.equal(mission.status, "completed");
    assert.equal(mission.integration_sha, item.mission.integration_sha);
    const scheduler = await get(item.project, `runs/${mission.run_id}/scheduler`);
    assert.equal(scheduler.deadlineSeconds, scheduler.policy.max_duration_secs);
    assert.equal(scheduler.activeClaims, 0);
    assert.equal(scheduler.ownerActive, false);
    assert.ok(scheduler.integrations.some(record => record.status === "published"));
    if (name === "Conflict") { assert.ok(scheduler.integrations.some(record => record.status === "conflict")); assert.ok(scheduler.revisions.some(revision => revision.source === "replan" && revision.plannerSessionId)); }
    await select(item); await view("Orchestrator");
    await page.locator(".chat-panel:visible").first().getByLabel("Conversation", { exact: true }).selectOption(item.conversation);
    await page.getByText(`Workflow #${mission.run_id} is completed.`, { exact: false }).waitFor();
    await view("Progress");
    const history = page.locator('.chat-progress:visible details[aria-label="Plan revision history"]').first();
    if (!(await history.getAttribute("open"))) await history.locator("summary").click();
    assert.ok((await history.innerText()).includes("Revision 1"));
    if (name === "Conflict") assert.ok((await history.innerText()).includes("Revision 2"));
    await shot(name.toLowerCase() + "-history-restored");
    await page.getByRole("button", { name: "Open workflow and review", exact: true }).last().click();
    await page.getByText(`${mission.completed} of ${mission.total} current tasks completed`, { exact: false }).waitFor();
    const evidence = page.locator(".ws-attempt-evidence:visible").filter({ has: page.locator("summary", { hasText: /approved$/ }) }).last();
    if ((await evidence.getAttribute("open")) === null) await evidence.locator("summary").click();
    await evidence.getByLabel(/Captured diff for task/).scrollIntoViewIfNeeded();
    assert.match(await evidence.innerText(), /Reviewer: approve/);
    await shot(name.toLowerCase() + "-approved-evidence");
  });
  await test("Rejected semantic candidate and provider refusal remain truthful after restart", async () => {
    const item = acceptance.scenarios.Repair;
    const missions = await get(item.project, `conversations/${item.conversation}/missions`);
    assert.equal(missions[0].status, "failed");
    assert.equal(missions[0].integration_sha, null);
    assert.ok(missions[0].integrations.some(record => record.status === "validation_failed"));
    await select(item); await view("Orchestrator");
    await page.locator(".chat-panel:visible").first().getByLabel("Conversation", { exact: true }).selectOption(item.conversation);
    await page.getByText(`Workflow #${missions[0].run_id} is failed.`, { exact: false }).waitFor();
    await view("Progress"); await shot("failed-candidate-provider-refusal");
  });
  await test("Project identity and replay cursor cannot resolve another conversation", async () => {
    const a = acceptance.scenarios.Parallel, b = acceptance.scenarios.Conflict;
    const response = await context.request.get(`${base}/api/projects/${b.project}/conversations/${a.conversation}`);
    assert.equal(response.status(), 404);
    const stream = await page.evaluate(async url => {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 10000);
      try { const response = await fetch(url, { signal: controller.signal }); const reader = response.body.getReader(); const decoder = new TextDecoder(); let text = ""; while (text.length < 65536 && !text.includes("event: conversation")) { const chunk = await reader.read(); if (chunk.done) break; text += decoder.decode(chunk.value, { stream: true }); } await reader.cancel(); return { status: response.status, text }; }
      finally { clearTimeout(timer); controller.abort(); }
    }, `${base}/api/projects/${a.project}/conversations/${a.conversation}/events?after=0`);
    assert.equal(stream.status, 200);
    const text = stream.text;
    assert.match(text, /event: conversation/);
    assert.ok(!text.includes(b.conversation));
  });
  await test("Actual task and integration roots open read-only in CodeMirror", async () => {
    const item = acceptance.scenarios.Parallel;
    await select(item);
    const roots = (await get(item.project, "files/roots")).roots;
    for (const selected of [roots.find(root => root.kind === "task_worktree"), roots.find(root => root.kind === "integration_snapshot")]) {
      assert.ok(selected?.read_only);
      await view("Explorer");
      const explorer = page.locator(".files-explorer:visible").first();
      await explorer.getByLabel("Filesystem root", { exact: true }).selectOption(selected.id);
      const tree = explorer.getByRole("tree");
      await until(async () => (await tree.getAttribute("aria-busy")) === "false", "root listing");
      const source = tree.getByRole("treeitem").filter({ has: page.locator(".files-tree-name", { hasText: /^src$/ }) }).first();
      if ((await source.getAttribute("aria-expanded")) !== "true") await source.click();
      const file = tree.getByRole("treeitem").filter({ has: page.locator(".files-tree-name", { hasText: /^validation\.py$/ }) }).first();
      await file.click();
      const editor = page.locator(".files-editor:visible").last();
      await editor.locator(".cm-editor").waitFor();
      assert.equal(await editor.locator(".cm-content").getAttribute("contenteditable"), "false");
      assert.match(await editor.innerText(), /read.only/i);
      assert.match(await editor.locator(".cm-content").innerText(), /def validate_request/);
      await shot(selected.kind + "-editor");
    }
  });
  await test("Correlated V1 Agent Graph renders superseded tasks and Settings", async () => {
    const legacy = await context.newPage(); legacy.on("pageerror", error => result.errors.push(String(error)));
    await legacy.goto(base + "/?project=" + acceptance.scenarios.Conflict.project);
    await legacy.getByRole("table").waitFor();
    await legacy.getByRole("button", { name: "Agent Graph", exact: true }).click();
    await legacy.locator(".react-flow").waitFor();
    await legacy.screenshot({ path: path.join(output, "screenshots/v1-superseded.png") });
    result.screenshots.push("v1-superseded");
    await legacy.getByRole("button", { name: "Settings", exact: true }).click();
    await legacy.getByRole("heading", { name: "Settings", exact: true }).waitFor(); await legacy.close();
  });
  await test("Light, dark, narrow workspace and browser reload restore real mission state", async () => {
    await select(acceptance.scenarios.Conflict); await view("Orchestrator"); await view("Progress");
    const saved = () => page.locator(".ws-status").getByRole("status").filter({ hasText: /^Workspace saved$/ }).waitFor();
    await page.getByLabel("Theme", { exact: true }).selectOption("light"); await saved(); await shot("light");
    await page.getByLabel("Theme", { exact: true }).selectOption("dark"); await saved(); await shot("dark");
    await page.reload(); await page.getByLabel("Open registered project").waitFor();
    await saved();
    assert.equal(await page.locator(`#project-${acceptance.scenarios.Conflict.project}`).getAttribute("aria-selected"), "true");
    assert.equal(await page.getByLabel("Theme", { exact: true }).inputValue(), "dark");
    await page.setViewportSize({ width: 1024, height: 768 }); await saved(); await shot("narrow-restored");
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    result.browserHeap = await page.evaluate(() => performance.memory?.usedJSHeapSize ?? null);
  });
  await test("All served production assets match the final frontend build", async () => {
    async function visit(folder) { for (const entry of await fs.readdir(folder, { withFileTypes: true })) { const file = path.join(folder, entry.name); if (entry.isDirectory()) await visit(file); else { const relative = path.relative(path.join(root, "apps/dashboard/dist"), file).replaceAll("\\", "/"); const response = await context.request.get(`${base}/${relative}`); assert.ok(response.ok()); const expected = hash(await fs.readFile(file)); assert.equal(hash(await response.body()), expected, relative); result.assets.push({ path: relative, sha256: expected }); } } }
    await visit(path.join(root, "apps/dashboard/dist"));
  });
  assert.deepEqual(result.errors, []);
  result.status = "PASS";
} catch (error) { result.status = "FAIL"; result.failure = String(error); console.error(error); await page?.screenshot({ path: path.join(output, "screenshots/failure.png") }).catch(() => {}); process.exitCode = 1; }
finally { result.finished = new Date().toISOString(); await fs.writeFile(path.join(output, "results.json"), JSON.stringify(result, null, 2)); await browser?.close(); if (child.exitCode === null) { child.kill(); await new Promise(resolve => child.once("exit", resolve)); } }
