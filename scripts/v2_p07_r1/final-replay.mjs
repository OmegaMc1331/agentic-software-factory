import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p07-r1");
try {
  await fs.copyFile(
    path.join(output, "final-replay.json"),
    path.join(output, `final-replay-attempt-${Date.now()}.json`),
  );
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const acceptance = JSON.parse(
  await fs.readFile(path.join(output, "acceptance-results.json"), "utf8"),
);
const binary = path.join(output, "run/factory-final.exe");
const { chromium } = createRequire(
  path.join(root, "apps/dashboard/package.json"),
)("playwright");
const hash = (data) => createHash("sha256").update(data).digest("hex");
const result = {
  kind: "Final rebuilt application durable replay and asset verification, not new model execution",
  started: new Date().toISOString(),
  binary,
  binarySha256: hash(await fs.readFile(binary)),
  modelExecutionBinarySha256: acceptance.binarySha256,
  tests: [],
  assets: [],
  screenshots: [],
  errors: [],
};
const port = await new Promise((resolve) => {
  const s = net.createServer();
  s.listen(0, "127.0.0.1", () => {
    const p = s.address().port;
    s.close(() => resolve(p));
  });
});
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(path.join(output, "final-replay-server.log"), "w");
const server = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    acceptance.registry,
    "--allow-project-root",
    acceptance.fixture,
  ],
  {
    cwd: acceptance.pathways.Chat.root,
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  },
);
await log.close();
let browser, context, page;
async function until(fn) {
  for (let n = 0; n < 400; n++) {
    const value = await fn();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("Final replay wait exceeded 40 seconds");
}
async function test(name, fn) {
  await fn();
  result.tests.push({ name, status: "passed" });
  console.log(`PASS final build: ${name}`);
}
try {
  await until(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  });
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
    colorScheme: "dark",
  });
  const auth = await context.request.post(base + "/api/auth/session", {
    headers: { Origin: base },
    data: {},
  });
  assert.equal(auth.status(), 200);
  const csrf = (await auth.json()).csrfToken;
  async function get(project, resource) {
    const r = await context.request.get(
      `${base}/api/projects/${project}/${resource}`,
    );
    assert.ok(r.ok(), await r.text());
    return r.json();
  }
  page = await context.newPage();
  page.on("pageerror", (error) => result.errors.push(error.message));
  await page.goto(base + "/workspace.html");
  for (const [name, item] of Object.entries(acceptance.pathways))
    await test(`${name}: persisted result, approved diff and integration survive restart`, async () => {
      const opened = await context.request.post(
        `${base}/api/projects/${item.project}/open`,
        { headers: { Origin: base, "X-Factory-CSRF": csrf }, data: {} },
      );
      assert.ok(opened.ok(), await opened.text());
      const missions = await get(
        item.project,
        `conversations/${item.conversation}/missions`,
      );
      assert.equal(
        missions.find((mission) => mission.run_id === item.mission.run_id)
          ?.integration_sha,
        item.mission.integration_sha,
      );
      const detail = await get(item.project, `runs/${item.mission.run_id}`);
      assert.equal(detail.run.status, "completed");
      assert.ok(
        detail.attempts.some(
          (attempt) =>
            attempt.status === "approved" &&
            attempt.review?.decision === "approve" &&
            attempt.evidence?.diffPatch?.includes("return"),
        ),
      );
      const rows = await get(
        item.project,
        `conversations/${item.conversation}/messages`,
      );
      assert.ok(
        rows.some(
          (message) =>
            message.id === item.command.response_id &&
            message.content.includes("is completed"),
        ),
      );
      await page
        .getByLabel("Open registered project")
        .selectOption(item.project);
      await page.getByRole("button", { name: "Views", exact: true }).click();
      await page
        .getByRole("menuitem", { name: "Orchestrator", exact: true })
        .click();
      const chat = page.locator(".chat-panel:visible").first();
      await chat
        .getByLabel("Conversation", { exact: true })
        .selectOption(item.conversation);
      await chat
        .getByText(`Workflow #${item.mission.run_id} is completed.`, {
          exact: false,
        })
        .waitFor();
      await chat
        .getByRole("button", { name: "Evidence and review", exact: true })
        .last()
        .click();
      const evidence = page
        .locator(".ws-attempt-evidence:visible")
        .filter({ has: page.locator("summary", { hasText: / · approved$/ }) })
        .last();
      if ((await evidence.getAttribute("open")) === null)
        await evidence.locator("summary").click();
      await evidence
        .getByLabel(/Captured diff for task/)
        .scrollIntoViewIfNeeded();
      assert.match(await evidence.innerText(), /Reviewer: approve/);
      const filename = `final-${name.toLowerCase()}-replay.png`;
      await page.screenshot({
        path: path.join(output, "screenshots", filename),
      });
      result.screenshots.push(filename);
    });
  await test("Conversation identity cannot resolve through another project's matching run ID", async () => {
    const r = await context.request.get(
      `${base}/api/projects/${acceptance.pathways.Visual.project}/conversations/${acceptance.pathways.Chat.conversation}`,
    );
    assert.equal(r.status(), 404);
  });
  await test("Every served production asset matches the final frontend build", async () => {
    async function visit(folder) {
      for (const item of await fs.readdir(folder, { withFileTypes: true })) {
        const file = path.join(folder, item.name);
        if (item.isDirectory()) await visit(file);
        else {
          const relative = path
            .relative(path.join(root, "apps/dashboard/dist"), file)
            .replaceAll("\\", "/");
          const response = await context.request.get(`${base}/${relative}`);
          assert.ok(response.ok());
          const expected = hash(await fs.readFile(file));
          assert.equal(hash(await response.body()), expected, relative);
          result.assets.push({ path: relative, sha256: expected });
        }
      }
    }
    await visit(path.join(root, "apps/dashboard/dist"));
  });
  await test("Legacy V1 route remains served by the rebuilt application", async () => {
    const r = await context.request.get(base + "/");
    assert.equal(r.status(), 200);
    assert.match(await r.text(), /Factory/);
  });
  assert.deepEqual(result.errors, []);
} catch (error) {
  result.failure = String(error);
  throw error;
} finally {
  result.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "final-replay.json"),
    JSON.stringify(result, null, 2),
  );
  await context?.close();
  await browser?.close();
  if (server.exitCode === null)
    await new Promise((resolve) => {
      server.once("exit", resolve);
      server.kill();
    });
}
