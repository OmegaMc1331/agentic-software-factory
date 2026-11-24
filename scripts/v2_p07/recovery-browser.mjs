import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
const root = path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../..",
  ),
  output = path.resolve(
    root,
    process.env.ASF_P07_OUTPUT_DIR ?? ".factory/v2/p07",
  );
const original = JSON.parse(
  await fs.readFile(path.join(output, "browser-results.json"), "utf8"),
);
const require = createRequire(path.join(root, "apps/dashboard/package.json")),
  { chromium } = require("playwright");
const port = await new Promise((resolve) => {
    const server = net.createServer();
    server.listen(0, "127.0.0.1", () => {
      const port = server.address().port;
      server.close(() => resolve(port));
    });
  }),
  base = `http://127.0.0.1:${port}`;
const binary = process.env.ASF_P07_BINARY ?? original.binary;
const result = {
  schema: 1,
  binary,
  binarySha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  scriptSha256: createHash("sha256")
    .update(await fs.readFile(fileURLToPath(import.meta.url)))
    .digest("hex"),
  started: new Date().toISOString(),
  tests: [],
  screenshots: [],
};
const log = await fs.open(path.join(output, "recovery-server.log"), "w");
const child = spawn(
  binary,
  [
    "start",
    "--no-browser",
    "--port",
    String(port),
    "--registry",
    original.registry,
    "--allow-project-root",
    original.fixture,
  ],
  {
    cwd: path.join(original.fixture, "Project A"),
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  },
);
await log.close();
let browser, context, page, id;
async function eventually(fn) {
  const until = Date.now() + 20000;
  while (Date.now() < until) {
    if (await fn()) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("Recovery browser condition timed out");
}
async function auth() {
  const response = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(response.ok());
  return (await response.json()).csrfToken;
}
async function messages() {
  const response = await context.request.get(
    `${base}/api/projects/${original.projects.c}/conversations/${id}/messages`,
  );
  assert.ok(response.ok());
  return response.json();
}
async function test(name, fn) {
  try {
    await fn();
    result.tests.push({ name, status: "passed" });
    console.log(`PASS ${name}`);
  } catch (error) {
    result.tests.push({ name, status: "failed", error: String(error) });
    throw error;
  } finally {
    await fs.writeFile(
      path.join(output, "recovery-browser-results.json"),
      JSON.stringify(result, null, 2),
    );
  }
}
try {
  await eventually(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
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
  });
  const csrf = await auth();
  const opened = await context.request.post(
    `${base}/api/projects/${original.projects.c}/open`,
    { headers: { Origin: base, "X-Factory-CSRF": csrf }, data: {} },
  );
  assert.ok(opened.ok());
  page = await context.newPage();
  await page.goto(`${base}/workspace.html`);
  await page
    .getByLabel("Open registered project")
    .selectOption(original.projects.c);
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page
    .getByRole("menuitem", { name: "Orchestrator", exact: true })
    .click();
  const panel = page.locator(".chat-panel:visible").first(),
    composer = panel.getByLabel("Message to orchestrator");
  const creation = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      /\/conversations$/.test(new URL(response.url()).pathname),
  );
  await panel
    .getByRole("button", { name: "New conversation", exact: true })
    .click();
  const created = await creation;
  assert.ok(created.ok());
  id = (await created.json()).id;
  await eventually(
    async () =>
      (await panel.getByLabel("Conversation", { exact: true }).inputValue()) ===
      id,
  );
  await composer.waitFor();
  const endpoint = `${base}/api/projects/${original.projects.c}/conversations/${id}/messages`;
  await test("composer Shift-Enter inserts a newline and Enter submits once", async () => {
    await composer.fill("Explain this question");
    await composer.press("Shift+Enter");
    await composer.type("without starting a mission.");
    assert.ok((await composer.inputValue()).includes("\n"));
    const accepted = page.waitForResponse(
      (response) =>
        response.url() === endpoint && response.request().method() === "POST",
    );
    await composer.press("Enter");
    assert.ok((await accepted).ok());
    await eventually(async () => {
      const value = await messages();
      return (
        value.length === 2 &&
        value.some(
          (message) =>
            message.role === "orchestrator" && message.status === "finished",
        )
      );
    });
    assert.equal(
      (await messages()).filter((message) => message.role === "user").length,
      1,
    );
  });
  await test("lost POST acknowledgment retains text and Retry reconciles one durable command", async () => {
    await page.route(endpoint, async (route) => {
      if (route.request().method() === "POST") {
        const response = await route.fetch();
        assert.ok(response.ok());
        await route.abort("failed");
      } else await route.continue();
    });
    const text = "Explain this second question without execution.";
    await composer.fill(text);
    await panel.getByRole("button", { name: "Send", exact: true }).click();
    await panel
      .getByRole("button", { name: "Retry submission", exact: true })
      .waitFor();
    assert.equal(await composer.inputValue(), text);
    await eventually(async () => {
      const value = await messages();
      return (
        value.length === 4 &&
        value.every((message) => message.status === "finished")
      );
    });
    await page.unroute(endpoint);
    const repeated = page.waitForResponse(
      (response) =>
        response.url() === endpoint && response.request().method() === "POST",
    );
    await panel
      .getByRole("button", { name: "Retry submission", exact: true })
      .click();
    assert.ok((await repeated).ok());
    await eventually(async () => (await composer.inputValue()) === "");
    assert.equal((await messages()).length, 4);
  });
  await test("an unsent composer draft survives reload without a backend message", async () => {
    const draft = "Unsent local composer draft; do not execute.";
    await composer.fill(draft);
    await eventually(
      async () =>
        await page.evaluate(
          (text) =>
            Object.keys(localStorage).some(
              (key) =>
                key.startsWith("asf.v2.composer.1:") &&
                localStorage.getItem(key)?.includes(text),
            ),
          draft,
        ),
    );
    await eventually(async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    );
    await page.reload();
    await eventually(
      async () =>
        (await page
          .locator(".chat-panel:visible")
          .first()
          .getByLabel("Message to orchestrator")
          .inputValue()) === draft,
    );
    assert.equal((await messages()).length, 4);
    await page.screenshot({
      path: path.join(output, "screenshots/keyboard-recovery.png"),
    });
    result.screenshots.push("keyboard-recovery.png");
  });
  await panel.getByRole("button", { name: "Visual plan", exact: true }).click();
  await panel
    .getByLabel("Mission objective")
    .fill("Draft visual plan - not submitted");
  await panel.getByRole("button", { name: "Add task", exact: true }).click();
  await panel
    .getByLabel("Title T1", { exact: true })
    .fill("Draft inspection task");
  const graph = panel.locator(".chat-plan-graph");
  await graph.scrollIntoViewIfNeeded();
  await graph.locator(".react-flow__node").first().waitFor();
  await page.screenshot({
    path: path.join(output, "screenshots/visual-plan-overview.png"),
  });
  result.screenshots.push("visual-plan-overview.png");
  await test("planning and execution captures follow actual gated backend phases", async () => {
    const projectRoot = path.join(original.fixture, "Controlled C"),
      config = path.join(projectRoot, ".factory/config.toml"),
      previous = await fs.readFile(config, "utf8"),
      gateRoot = await fs.mkdtemp(path.join(output, "phase-gates-")),
      plannerGate = path.join(gateRoot, "plan-release"),
      workerGate = path.join(gateRoot, "worker-release"),
      shim = path.join(gateRoot, "planner.mjs"),
      agent = path.join(
        original.fixture,
        process.platform === "win32"
          ? "controlled-agent.exe"
          : "controlled-agent",
      );
    await fs.writeFile(
      shim,
      `import fs from 'node:fs/promises';
import {spawn} from 'node:child_process';
const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
const prompt=Buffer.concat(chunks);
if(!prompt.toString('utf8').startsWith('ASF_ORCHESTRATOR_V1')) {
 while(!(await fs.stat(${JSON.stringify(plannerGate)}).catch(()=>false)))await new Promise(r=>setTimeout(r,20));
}
const child=spawn(${JSON.stringify(agent)},['planner'],{stdio:['pipe','inherit','inherit'],windowsHide:true});
child.stdin.end(prompt);child.on('error',()=>process.exit(1));child.on('exit',code=>process.exit(code??1));
`,
    );
    const profiles = ["planner", "worker", "reviewer"]
      .map(
        (role) =>
          `[agents.${role}]\nkind = "custom"\ncommand = ${JSON.stringify(role === "planner" ? process.execPath : agent)}\nargs = ${JSON.stringify(role === "planner" ? [shim] : [role])}\n${role === "worker" ? `[agents.worker.env]\nP07_WAIT = ${JSON.stringify(workerGate)}\n` : ""}[[role_assignments]]\nrole = "${role}"\nagent = "${role}"\npreferred = true\n`,
      )
      .join("\n");
    const missionsPath = `${base}/api/projects/${original.projects.c}/conversations/`;
    try {
      await fs.writeFile(config, profiles);
      await page.reload();
      const recoverLayout = page.getByRole("button", {
        name: "Save independent copy",
        exact: true,
      });
      if (await recoverLayout.isVisible()) {
        await recoverLayout.click();
        await eventually(async () => !(await recoverLayout.isVisible()));
        result.phaseLayoutRecovered = true;
      }
      const creation = page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          /\/conversations$/.test(new URL(response.url()).pathname),
      );
      await panel
        .getByRole("button", { name: "New conversation", exact: true })
        .click();
      const created = await creation;
      assert.ok(created.ok());
      id = (await created.json()).id;
      await eventually(
        async () =>
          (await panel
            .getByLabel("Conversation", { exact: true })
            .inputValue()) === id,
      );
      await composer.fill("START_MISSION");
      await composer.press("Enter");
      await page.getByRole("button", { name: "Views", exact: true }).click();
      await page
        .getByRole("menuitem", { name: "Progress", exact: true })
        .click();
      const progress = page.locator(".chat-progress:visible").first();
      const readMission = async () => {
        const response = await context.request.get(
          `${missionsPath}${id}/missions`,
        );
        assert.ok(response.ok());
        return (await response.json())[0];
      };
      let mission;
      await eventually(
        async () => (mission = await readMission())?.status === "planning",
      );
      result.phaseMission = {
        project: original.projects.c,
        conversation: id,
        run_id: mission.run_id,
      };
      await progress
        .getByText(`#${mission.run_id} · planning`, { exact: true })
        .waitFor();
      await page.screenshot({
        path: path.join(output, "screenshots/mission-planning.png"),
      });
      await fs.writeFile(plannerGate, "release");
      await eventually(async () =>
        (mission = await readMission())?.sessions.some(
          (session) =>
            session.role === "worker" && session.status === "running",
        ),
      );
      await progress
        .getByText(`#${mission.run_id} · active`, { exact: true })
        .waitFor();
      await page.screenshot({
        path: path.join(output, "screenshots/mission-running.png"),
      });
      await fs.writeFile(workerGate, "release");
      await eventually(
        async () => (mission = await readMission())?.status === "completed",
      );
      assert.equal(mission.completed, 1);
      assert.equal(mission.total, 1);
      await progress
        .getByText(`#${mission.run_id} · completed`, { exact: true })
        .waitFor();
      await eventually(async () =>
        (await messages()).some(
          (message) =>
            message.role === "orchestrator" &&
            message.run_id === mission.run_id &&
            message.status === "finished",
        ),
      );
      await panel
        .getByText(`Workflow #${mission.run_id} is completed.`, {
          exact: false,
        })
        .waitFor();
      await progress
        .getByText("Tasks, agents and evidence", { exact: true })
        .click();
      await page
        .locator(".ws-list button")
        .filter({ has: page.getByText(`#${mission.run_id}`, { exact: true }) })
        .getByText("Run status: completed · 1 recorded tasks", { exact: true })
        .waitFor();
      await page.screenshot({
        path: path.join(output, "screenshots/progress-details.png"),
      });
      result.screenshots.push(
        "mission-planning.png",
        "mission-running.png",
        "progress-details.png",
      );
    } finally {
      await fs.writeFile(plannerGate, "release");
      await fs.writeFile(workerGate, "release");
      await fs.writeFile(config, previous);
    }
  });
  result.finished = new Date().toISOString();
} finally {
  await fs.writeFile(
    path.join(output, "recovery-browser-results.json"),
    JSON.stringify(result, null, 2),
  );
  await context?.close();
  await browser?.close();
  if (child.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
}
