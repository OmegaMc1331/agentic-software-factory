import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const local = path.join(root, ".factory/v2/p09");
const native = JSON.parse(
  await fs.readFile(path.join(local, "acceptance-final/results.json"), "utf8"),
);
const output = path.join(local, "replay");
const binary = process.env.ASF_P09_BINARY;
assert.ok(binary);
const item = native.projects.Native;
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const hash = (value) => createHash("sha256").update(value).digest("hex");
const result = {
  schema: 1,
  kind: "Model-free production browser inspection of actual P09 native results",
  started: new Date().toISOString(),
  binary,
  binarySha256: hash(await fs.readFile(binary)),
  nativeBinarySha256: native.binarySha256,
  sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  }).stdout.trim(),
  tests: [],
  screenshots: [],
  assets: [],
  pageErrors: [],
  modelInvocations: 0,
};
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
try {
  await fs.copyFile(
    path.join(output, "results.json"),
    path.join(output, `attempt-${Date.now()}.json`),
  );
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const persist = () =>
  fs.writeFile(
    path.join(output, "results.json"),
    JSON.stringify(result, null, 2) + "\n",
  );
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(fn, label, ms = 30000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await delay(100);
  }
  throw new Error("Timeout: " + label);
}
async function test(name, action) {
  const start = performance.now();
  try {
    await action();
    result.tests.push({
      name,
      status: "passed",
      ms: performance.now() - start,
    });
  } catch (error) {
    result.tests.push({
      name,
      status: "failed",
      error: String(error),
      ms: performance.now() - start,
    });
    throw error;
  } finally {
    await persist();
  }
}
let child, browser, context, page, base, csrf;
async function start() {
  const port = await new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
  base = `http://127.0.0.1:${port}`;
  const log = await fs.open(path.join(output, `server-${Date.now()}.log`), "w");
  child = spawn(
    binary,
    [
      "start",
      "--no-browser",
      "--port",
      String(port),
      "--max-active-tasks",
      "2",
      "--registry",
      path.join(native.fixture, "registry.sqlite3"),
      "--allow-project-root",
      native.fixture,
    ],
    { cwd: item.root, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
  );
  await log.close();
  await until(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  }, "server");
  browser = await chromium.launch({ channel: "msedge", headless: true });
  context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    reducedMotion: "reduce",
  });
  page = await context.newPage();
  page.on("pageerror", (error) => result.pageErrors.push(String(error)));
  const auth = await context.request.post(base + "/api/auth/session", {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(auth.ok());
  csrf = (await auth.json()).csrfToken;
  for (const p of Object.values(native.projects)) await mutate(p, "open", {});
}
async function get(project, suffix) {
  const r = await context.request.get(
    `${base}/api/projects/${project.project}/${suffix}`,
  );
  assert.ok(r.ok(), await r.text());
  return r.json();
}
async function mutate(project, suffix, data) {
  const r = await context.request.post(
    `${base}/api/projects/${project.project}/${suffix}`,
    { headers: { Origin: base, "X-Factory-CSRF": csrf }, data },
  );
  assert.ok(r.ok(), await r.text());
  return r.json();
}
async function view(name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
async function saved() {
  await until(
    async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    "durable workspace save",
  );
}
async function shot(name, element) {
  const destination = path.join(output, "screenshots", name + ".png");
  if (element) await element.screenshot({ path: destination });
  else await page.screenshot({ path: destination, fullPage: true });
  if (!result.screenshots.includes(name)) result.screenshots.push(name);
  await persist();
}
async function maximize() {
  await page
    .getByRole("button", { name: "Layout actions", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "Toggle maximization", exact: true })
    .click();
}
try {
  await start();
  await test("Restart retains the completed native mission, exact integration and immutable snapshots", async () => {
    const detail = await get(item, `runs/${item.run}`);
    assert.equal(detail.run.status, "completed");
    assert.equal(detail.integration.head, item.integrationCommit);
    assert.equal(detail.sessions.length, 6);
    for (const session of detail.sessions) {
      const snapshot = await get(
        item,
        `agent-sessions/${session.id}/execution-snapshot`,
      );
      assert.equal(snapshot.profile.revision, 1);
      assert.equal(snapshot.requestedModel, "gpt-6.1-sol");
      const observed = await get(
        item,
        `agent-sessions/${session.id}/model-observation`,
      );
      assert.equal(observed.model, "gpt-6.1-sol");
    }
    const state = await get(item, `runs/${item.run}/scheduler`);
    assert.equal(state.activeClaims, 0);
    assert.equal(
      state.integrations.filter((r) => r.status === "published").length,
      2,
    );
    result.recoveredState = {
      run: detail.run,
      integrations: state.integrations,
      sessionCount: detail.sessions.length,
    };
  });
  await test("Completed handoff retries are idempotent and cannot launch another Worker", async () => {
    const handoff = item.handoffs.find((h) => h.state === "completed");
    const body = {
      toProfileId: handoff.target.id,
      expectedProfileRevision: handoff.target.revision,
      idempotencyKey: handoff.idempotencyKey,
      reason: handoff.reason,
    };
    const replies = await Promise.all(
      [1, 2].map(() =>
        mutate(item, `runs/${item.run}/tasks/${handoff.taskId}/handoff`, body),
      ),
    );
    assert.ok(
      replies.every((h) => h.id === handoff.id && h.state === "completed"),
    );
    assert.equal((await get(item, `runs/${item.run}`)).sessions.length, 6);
  });
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").waitFor();
  await page.getByLabel("Open registered project").selectOption(item.project);
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await test("Actual conversation result and handoff history restore without model execution", async () => {
    await view("Orchestrator");
    await page
      .getByLabel("Conversation", { exact: true })
      .selectOption(item.conversation);
    await until(
      async () =>
        (await page.locator(".chat-panel:visible").last().innerText()).includes(
          "2 of 2 tasks completed",
        ),
      "actual completion summary",
    );
    await saved();
    await shot("final-conversation-result");
    await view("Progress");
    const handoffs = page.locator(".asf-handoffs:visible").last();
    if ((await handoffs.getAttribute("open")) === null)
      await handoffs.locator("summary").first().click();
    await handoffs.getByText("Task #1 · completed", { exact: true }).waitFor();
    await handoffs.scrollIntoViewIfNeeded();
    await saved();
    await shot("final-completed-handoff");
  });
  await test("Replacement and original consoles show their own historical configurations", async () => {
    const handoffs = page.locator(".asf-handoffs:visible").last();
    await handoffs
      .getByRole("button", { name: "Replacement session #4", exact: true })
      .click();
    await maximize();
    let snapshot = page.locator(".asf-execution-snapshot:visible").last();
    await snapshot.locator("summary").waitFor();
    await snapshot.locator("summary").click();
    assert.match(await snapshot.innerText(), /Implementation B/);
    assert.match(await snapshot.innerText(), /medium/);
    assert.ok((await snapshot.locator("pre").boundingBox()).height <= 322);
    await shot("final-replacement-snapshot", snapshot);
    await maximize();
    await view("Progress");
    const history = page.locator(".asf-handoffs:visible").last();
    if ((await history.getAttribute("open")) === null)
      await history.locator("summary").first().click();
    await history
      .getByRole("region", {
        name: `Handoff ${item.handoffs.find((h) => h.state === "completed").id}`,
        exact: true,
      })
      .getByRole("button", { name: "Original session #2", exact: true })
      .click();
    await maximize();
    snapshot = page.locator(".asf-execution-snapshot:visible").last();
    await snapshot.locator("summary").waitFor();
    await snapshot.locator("summary").click();
    assert.match(await snapshot.innerText(), /Implementation A/);
    assert.match(await snapshot.innerText(), /revision 1/);
    assert.match(await snapshot.innerText(), /low/);
    await shot("final-original-snapshot", snapshot);
    await maximize();
  });
  await test("Actual nonempty task patch and genuine approval remain inspectable", async () => {
    await view("Progress");
    await page
      .getByRole("button", { name: "Open workflow and review", exact: true })
      .first()
      .click();
    await maximize();
    const attempt = page
      .locator(".ws-attempt-evidence:visible")
      .filter({ hasText: /approved/ })
      .last();
    await attempt.locator("summary").click();
    await attempt
      .locator("p")
      .filter({ hasText: /^Reviewer: approve/ })
      .first()
      .waitFor();
    assert.match(await attempt.innerText(), /Patch SHA-256/);
    assert.match(
      await attempt.locator("pre").innerText(),
      /src\/math_tools\.py|src\/events\.py/,
    );
    await shot("final-patch-and-independent-review");
    await maximize();
  });
  await test("Enable, edit and archive preserve original session identity and revision history", async () => {
    await view("Agents");
    let details = await get(item, `agent-profiles/${native.profiles.a.id}`);
    const host = page.locator('[aria-label="Agent management"]:visible').last();
    const row = () =>
      host.locator(".asf-agent-row").filter({
        has: page.locator("small", { hasText: native.profiles.a.id }),
      });
    if (!details.profile.archived) {
      if (!details.profile.enabled) {
        await row()
          .getByRole("button", { name: "Enable", exact: true })
          .click();
        await until(
          async () =>
            (await get(item, `agent-profiles/${native.profiles.a.id}`)).profile
              .enabled,
          "enabled",
        );
        await row()
          .getByRole("button", { name: "Disable", exact: true })
          .waitFor();
      }
      await row()
        .getByRole("button", { name: /^Edit / })
        .focus();
      await page.keyboard.press("Enter");
      const form = host.getByRole("form", { name: "Agent profile editor" });
      await form
        .getByLabel("Agent name", { exact: true })
        .fill("Implementation A archived after acceptance");
      await form
        .getByLabel("Agent instructions", { exact: true })
        .fill(
          "Changed after execution. Historical attempts retain their original configuration.",
        );
      await form
        .getByRole("button", { name: "Save agent profile", exact: true })
        .click();
      await row()
        .getByRole("button", {
          name: "Edit Implementation A archived after acceptance",
          exact: true,
        })
        .waitFor();
      await row().getByRole("button", { name: "Archive", exact: true }).click();
      await until(
        async () =>
          (await get(item, `agent-profiles/${native.profiles.a.id}`)).profile
            .archived,
        "archived",
      );
    }
    await host.getByLabel("Include archived", { exact: true }).check();
    await row()
      .getByText(/Archived/)
      .waitFor();
    await row().locator("summary").click();
    await row().locator("pre").first().waitFor();
    await row().scrollIntoViewIfNeeded();
    await saved();
    await shot("final-archived-profile-history");
    const snapshot = await get(item, "agent-sessions/2/execution-snapshot");
    assert.equal(snapshot.profile.revision, 1);
    assert.equal(snapshot.profile.name, "Implementation A");
    assert.ok(
      (await get(item, `agent-profiles/${native.profiles.a.id}/revisions`))
        .length >= 4,
    );
    result.archivedProfile = (
      await get(item, `agent-profiles/${native.profiles.a.id}`)
    ).profile;
  });
  await test("Profiles with matching names remain project-owned at browser restore", async () => {
    await page
      .getByLabel("Open registered project")
      .selectOption(native.projects.Other.project);
    await view("Agents");
    const host = page.locator('[aria-label="Agent management"]:visible').last();
    await until(
      async () => (await host.locator(".asf-agent-row").count()) === 50,
      "fifty profiles",
    );
    await saved();
    await shot("final-fifty-profiles-dark");
    await page.getByLabel("Theme", { exact: true }).selectOption("light");
    await saved();
    await shot("final-fifty-profiles-light");
    await page.setViewportSize({ width: 1050, height: 850 });
    await saved();
    await shot("final-profiles-narrow");
    await page.setViewportSize({ width: 1440, height: 1000 });
    const denied = await context.request.get(
      `${base}/api/projects/${native.projects.Other.project}/agent-profiles/${native.profiles.a.id}`,
    );
    assert.equal(denied.status(), 404);
  });
  await test("Integrated source restores read-only in a usable CodeMirror group", async () => {
    await page.getByLabel("Open registered project").selectOption(item.project);
    await page.getByRole("button", { name: "Code", exact: true }).click();
    await view("Explorer");
    const roots = (await get(item, "files/roots")).roots;
    const integration = roots.find((r) => r.kind === "integration_snapshot");
    assert.ok(integration?.read_only);
    const explorer = page.locator(".files-explorer:visible").first();
    await explorer
      .getByLabel("Filesystem root", { exact: true })
      .selectOption(integration.id);
    const tree = explorer.getByRole("tree");
    const directory = tree
      .getByRole("treeitem")
      .filter({ has: page.locator(".files-tree-name", { hasText: /^src$/ }) })
      .first();
    if ((await directory.getAttribute("aria-expanded")) !== "true")
      await directory.click();
    await tree
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", { hasText: /^math_tools\.py$/ }),
      })
      .first()
      .click();
    const editor = page.locator(".files-editor:visible").last();
    await editor.locator(".cm-content").waitFor();
    assert.equal(
      await editor.locator(".cm-content").getAttribute("contenteditable"),
      "false",
    );
    assert.match(await editor.locator(".cm-content").innerText(), /isfinite/);
    await maximize();
    await saved();
    const box = await editor.boundingBox();
    assert.ok(box.width > 600 && box.height > 300, JSON.stringify(box));
    result.editorGeometry = box;
    await shot("final-integrated-editor-light");
    await page.getByLabel("Theme", { exact: true }).selectOption("dark");
    await saved();
    await shot("final-integrated-editor-dark");
    await page.reload();
    await page.getByLabel("Open registered project").waitFor();
    await saved();
    assert.match(
      await page.locator(".files-editor:visible").last().innerText(),
      /isfinite/,
    );
  });
  await test("Final embedded dashboard bodies match every built asset hash", async () => {
    async function visit(directory, prefix = "") {
      for (const e of await fs.readdir(directory, { withFileTypes: true })) {
        if (e.isDirectory())
          await visit(path.join(directory, e.name), prefix + e.name + "/");
        else {
          const name = prefix + e.name;
          const r = await context.request.get(base + "/" + name);
          assert.equal(r.status(), 200, name);
          const bytes = await fs.readFile(path.join(directory, e.name));
          assert.equal(hash(await r.body()), hash(bytes), name);
          result.assets.push({
            path: name,
            bytes: bytes.length,
            sha256: hash(bytes),
          });
        }
      }
    }
    await visit(path.join(root, "apps/dashboard/dist"));
  });
  assert.deepEqual(result.pageErrors, []);
  result.status = "PASS";
} catch (error) {
  result.status = "FAIL";
  result.failure = String(error);
  console.error(result.failure);
  process.exitCode = 1;
  await shot("final-replay-failure").catch(() => {});
} finally {
  result.finished = new Date().toISOString();
  await persist();
  if (browser) await browser.close();
  if (child && child.exitCode === null) {
    child.kill();
    await new Promise((resolve) => child.once("exit", resolve));
  }
}
