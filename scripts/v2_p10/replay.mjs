import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const source = JSON.parse(await fs.readFile(path.join(root, ".factory/v2/p10/native/results.json"), "utf8"));
assert.equal(source.status, "PASS");
const binary = process.env.ASF_P10_BINARY;
assert.ok(binary);
const output = process.env.ASF_P10_REPLAY ?? path.join(root, ".factory/v2/p10/replay");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const result = { schema: 1, kind: "Model-free inspection of retained genuine P10 results using final presentation", binary, binarySha256: createHash("sha256").update(await fs.readFile(binary)).digest("hex"), nativeBinarySha256: source.binarySha256, modelInvocations: 0, tests: [], screenshots: [], pageErrors: [] };
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium, request } = require("playwright");
const port = await new Promise((resolve) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); }); });
const base = `http://127.0.0.1:${port}`;
const handle = await fs.open(path.join(output, "server.log"), "w");
const server = spawn(binary, ["start", "--no-browser", "--port", String(port), "--registry", path.join(source.fixture, "registry.sqlite3"), "--allow-project-root", source.fixture], { cwd: source.projects.A.root, windowsHide: true, stdio: ["ignore", handle.fd, handle.fd] });
await handle.close();
let browser, inspector;
const delay = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(fn, name) { const end = Date.now() + 30000; while (Date.now() < end) { if (await fn()) return; await delay(200); } throw new Error(`Timed out: ${name}`); }
try {
  await until(async () => { try { return (await fetch(base + "/api/health")).ok; } catch { return false; } }, "owned replay startup");
  inspector = await request.newContext();
  const authentication = await inspector.post(base + "/api/auth/session", { headers: { Origin: base }, data: {} }); assert.ok(authentication.ok());
  const csrf = (await authentication.json()).csrfToken;
  async function api(item, suffix, method = "GET") { const response = await inspector.fetch(`${base}/api/projects/${item.projectId}/${suffix}`, { method, headers: { Origin: base, "X-Factory-CSRF": csrf } }); assert.ok(response.ok(), await response.text()); return response.json(); }
  const before = {};
  for (const [name, item] of Object.entries(source.projects)) {
    await api(item, "open", "POST");
    const detail = await api(item, `runs/${item.runId}`); assert.equal(detail.run.status, "completed"); before[name] = detail;
  }
  browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--disable-gpu", "--disable-extensions", "--disable-background-networking"] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  await context.addInitScript((id) => localStorage.setItem("asf.v2.workspace.last", id), source.workspaceId);
  const page = await context.newPage(); page.on("pageerror", (e) => result.pageErrors.push(String(e)));
  await page.goto(base + "/workspace.html"); await page.getByLabel("Open registered project").selectOption(source.projects.A.projectId); await page.getByRole("button", { name: "Focus", exact: true }).click();
  async function view(name) { await page.getByRole("button", { name: "Views", exact: true }).click(); await page.getByRole("menuitem", { name, exact: true }).click(); }
  async function saved() { await until(async () => (await page.locator(".ws-status").innerText()).includes("Workspace saved"), "workspace saved"); }
  async function shot(name) { await saved(); const file = `screenshots/${name}.png`; await page.screenshot({ path: path.join(output, file) }); result.screenshots.push({ file, sha256: createHash("sha256").update(await fs.readFile(path.join(output, file))).digest("hex") }); }
  await view("Orchestrator");
  const conversation = page.locator(".chat-panel:visible").last();
  await conversation.getByLabel("Conversation", { exact: true }).selectOption(String(source.projects.A.conversationId));
  await until(async () => (await conversation.locator(".chat-message").count()) >= source.projects.A.messages.length, "restored real message history");
  const recorded = conversation.locator(".chat-message").filter({ has: page.locator("header strong", { hasText: "Mission result" }) });
  assert.equal(await recorded.locator("header span").first().innerText(), "suspended");
  await recorded.first().scrollIntoViewIfNeeded(); await shot("historical-suspension-label-light");
  result.tests.push({ name: "Historical suspension uses recorded lifecycle without rewriting run outcome", status: "passed" });
  await view("Progress"); const progress = page.getByRole("region", { name: `Mission ${source.projects.A.runId} lifecycle`, exact: true });
  await progress.getByText("Execution: settled", { exact: true }).waitFor();
  assert.equal(await progress.getByRole("button", { name: "Validate and resume" }).count(), 0);
  assert.ok((await page.locator(".ws-list").first().innerText()).includes("Run status: completed"));
  await page.getByLabel("Theme", { exact: true }).selectOption("dark"); await shot("completed-lifecycle-dark");
  result.tests.push({ name: "Completed missions stay terminal with separate run and execution labels", status: "passed" });
  await saved(); await page.reload(); await page.getByLabel("Open registered project").waitFor(); await view("Orchestrator");
  await until(async () => (await page.locator(".chat-message:visible").count()) >= source.projects.A.messages.length, "reload without lost history");
  const ids = await page.locator(".chat-message:visible").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-message-id"))); assert.equal(new Set(ids).size, ids.length);
  await page.setViewportSize({ width: 1060, height: 800 }); await shot("restored-narrow"); result.tests.push({ name: "Reload keeps unique durable messages and compact layout", status: "passed" });
  await page.setViewportSize({ width: 1440, height: 1000 });
  const legacy = await context.newPage(); await legacy.goto(`${base}/?project=${source.projects.A.projectId}`); await legacy.getByRole("table").waitFor(); await legacy.close();
  result.tests.push({ name: "V1 retained workflow remains accessible", status: "passed" });
  for (const [name, item] of Object.entries(source.projects)) assert.deepEqual(await api(item, `runs/${item.runId}`), before[name]);
  result.tests.push({ name: "No new agents, attempts, reviews or integration operations during replay", status: "passed" });
  assert.deepEqual(result.pageErrors, []); result.status = "PASS";
} catch (error) { result.status = "FAILED"; result.error = String(error.stack ?? error); throw error; }
finally { await browser?.close(); await inspector?.dispose(); if (server.exitCode === null) { const exit = new Promise((r) => server.once("exit", r)); server.kill(); await exit; } await fs.writeFile(path.join(output, "results.json"), JSON.stringify(result, null, 2) + "\n"); }
