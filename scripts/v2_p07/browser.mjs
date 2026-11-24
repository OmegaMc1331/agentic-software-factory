import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.resolve(
  root,
  process.env.ASF_P07_OUTPUT_DIR ?? ".factory/v2/p07",
);
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const binary =
  process.env.ASF_P07_BINARY ??
  path.join(
    output,
    "run",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const fixture = await fs.mkdtemp(path.join(output, "browser-projects-"));
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p07-")),
  "registry.sqlite3",
);
const readOnlyQualification = process.env.ASF_P07_REAL_MODE === "read_only";
const controlledQualification = process.env.ASF_P07_REAL_MODE === "controlled";
const result = {
  schema: 1,
  realMissionMode: controlledQualification
    ? "controlled_process_mechanics_not_model_qualification"
    : readOnlyQualification
      ? "read_only_advisory"
      : "mutating",
  started: new Date().toISOString(),
  platform: process.platform,
  binary,
  binarySha256: createHash("sha256")
    .update(await fs.readFile(binary))
    .digest("hex"),
  fixture,
  registry,
  tests: [],
  screenshots: [],
  errors: [],
  measurements: [],
  realAgents: [],
  events: [],
};
const run = (program, args, cwd = root) => {
  const result = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
};
result.sourceCommit = run("git", ["rev-parse", "HEAD"]).trim();
result.scriptSha256 = createHash("sha256")
  .update(await fs.readFile(fileURLToPath(import.meta.url)))
  .digest("hex");
const codex = controlledQualification
  ? null
  : (process.env.ASF_P07_CODEX ??
    (process.platform === "win32"
      ? run("where.exe", ["codex.exe"]).trim().split(/\r?\n/)[0]
      : "codex"));
if (!controlledQualification)
  result.realAgents.push({
    command: codex,
    version: run(codex, ["--version"]).trim(),
    model: "configured CLI default, no model override",
  });
const controlled = path.join(
  fixture,
  process.platform === "win32" ? "controlled-agent.exe" : "controlled-agent",
);
run("rustc", [
  "--edition=2021",
  path.join(
    root,
    "crates/factory-conversation/tests/support/controlled_agent.rs",
  ),
  "-o",
  controlled,
]);
function configuration(real, extra = "") {
  return (
    real
      ? ["planner", "worker", "reviewer", "researcher"]
      : ["planner", "worker", "reviewer"]
  )
    .map(
      (role) =>
        `[agents.${role}]\nkind = "${real ? "codex" : "custom"}"\ncommand = ${JSON.stringify(real ? codex : controlled)}\nargs = ${JSON.stringify(real ? ["exec", "--ignore-user-config", "--ephemeral", "--disable", "apps", "--sandbox", role === "worker" && !readOnlyQualification ? "workspace-write" : "read-only", "--color", "never"] : [role])}\n${extra ? `[agents.${role}.env]\n${extra}\n` : ""}[[role_assignments]]\nrole = "${role}"\nagent = "${role}"\npreferred = true\n`,
    )
    .join("\n");
}
for (const name of ["Project A", "Project B", "Controlled C", "No agent D"]) {
  const folder = path.join(fixture, name);
  await fs.mkdir(folder);
  run("git", ["init", "-q", "-b", "main"], folder);
  run(binary, ["init"], folder);
  await fs.writeFile(path.join(folder, ".gitignore"), ".factory/\n.env\n");
  await fs.writeFile(
    path.join(folder, "source.py"),
    `def add(a, b):\n    return a + b\n\nPROJECT = ${JSON.stringify(name)}\n`,
  );
  run("git", ["add", "."], folder);
  run(
    "git",
    [
      "-c",
      "user.name=P07 Fixture",
      "-c",
      "user.email=p07@example.invalid",
      "commit",
      "-qm",
      "fixture",
    ],
    folder,
  );
  run("git", ["config", "user.name", "P07 Fixture"], folder);
  run("git", ["config", "user.email", "p07@example.invalid"], folder);
  await fs.writeFile(
    path.join(folder, ".factory/config.toml"),
    name === "No agent D"
      ? ""
      : configuration(name.startsWith("Project") && !controlledQualification),
  );
}
const port = await new Promise((resolve) => {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1", () => {
    const port = socket.address().port;
    socket.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
let child, browser, context, page, a, b, c, d, conversation;
async function eventually(work, label = "condition", timeout = 30000) {
  const until = Date.now() + timeout;
  let error;
  while (Date.now() < until) {
    try {
      const value = await work();
      if (value) return value;
    } catch (reason) {
      error = reason;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label}: ${error?.message ?? "deadline"}`);
}
async function start() {
  const fd = await fs.open(path.join(output, "browser-server.log"), "a");
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
      cwd: path.join(fixture, "Project A"),
      stdio: ["ignore", fd.fd, fd.fd],
      windowsHide: true,
    },
  );
  await fd.close();
  await eventually(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  }, "server startup");
}
async function stop() {
  if (child && child.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
}
async function auth() {
  const r = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(r.ok());
  return (await r.json()).csrfToken;
}
async function mutate(project, suffix, body = {}, method = "POST") {
  const csrf = await auth();
  const r = await context.request.fetch(
    `${base}/api/projects/${project}/${suffix}`,
    { method, headers: { Origin: base, "X-Factory-CSRF": csrf }, data: body },
  );
  assert.ok(r.ok(), `${r.status()} ${await r.text()}`);
  return r.json();
}
async function get(project, suffix) {
  const r = await context.request.get(
    `${base}/api/projects/${project}/${suffix}`,
  );
  assert.ok(r.ok(), `${r.status()} ${await r.text()}`);
  return r.json();
}
async function view(name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
async function select(project) {
  await page.getByLabel("Open registered project").selectOption(project);
  await page.locator(`#project-${project}[aria-selected=true]`).waitFor();
}
const panel = () => page.locator(".chat-panel:visible").first();
async function newConversation() {
  await view("Orchestrator");
  const pending = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      /\/api\/projects\/[^/]+\/conversations$/.test(
        new URL(response.url()).pathname,
      ),
  );
  await panel()
    .getByRole("button", { name: "New conversation", exact: true })
    .click();
  const response = await pending;
  assert.ok(response.ok(), await response.text());
  const created = await response.json();
  await eventually(
    async () =>
      (await panel()
        .getByLabel("Conversation", { exact: true })
        .inputValue()) === created.id,
    "new conversation selected",
  );
  await panel().getByLabel("Message to orchestrator").waitFor();
  return created.id;
}
const acceptedResponses = new Map();
async function submitClick(button) {
  const pending = page.waitForResponse(
    (response) =>
      response.request().method() === "POST" &&
      /\/conversations\/c_[a-f0-9]+\/messages$/.test(
        new URL(response.url()).pathname,
      ),
  );
  await button.click();
  const response = await pending;
  assert.ok(response.ok(), await response.text());
  const value = await response.json();
  acceptedResponses.set(value.conversation_id, value.response_id);
  return value;
}
async function send(text) {
  await panel().getByLabel("Message to orchestrator").fill(text);
  return submitClick(
    panel().getByRole("button", { name: "Send", exact: true }),
  );
}
async function waitResponse(project, id, timeout = 180000) {
  return eventually(
    async () => {
      const messages = await get(project, `conversations/${id}/messages`);
      const expected = acceptedResponses.get(id);
      const latest = messages.find(
        (m) => m.role === "orchestrator" && (!expected || m.id === expected),
      );
      return latest &&
        ["finished", "failed", "interrupted"].includes(latest.status)
        ? latest
        : false;
    },
    "agent response",
    timeout,
  );
}
async function shot(name) {
  await page.screenshot({
    path: path.join(output, "screenshots", `${name}.png`),
  });
  result.screenshots.push(`${name}.png`);
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
    console.log(`PASS ${name}`);
  } catch (error) {
    result.tests.push({ name, status: "failed", error: error.message });
    result.failureOptions = await page
      ?.locator(".chat-panel select option")
      .allTextContents()
      .catch(() => []);
    await page
      ?.screenshot({ path: path.join(output, "screenshots", "failure.png") })
      .catch(() => {});
    throw error;
  } finally {
    await fs.writeFile(
      path.join(output, "browser-results.json"),
      JSON.stringify(result, null, 2),
    );
  }
}
try {
  await start();
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  result.browser = browser.version();
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    colorScheme: "dark",
    reducedMotion: "reduce",
  });
  await auth();
  const projects = await (
    await context.request.get(`${base}/api/projects`)
  ).json();
  a = projects[0].project_id;
  for (const [name, set] of [
    ["Project B", (value) => (b = value)],
    ["Controlled C", (value) => (c = value)],
    ["No agent D", (value) => (d = value)],
  ]) {
    const csrf = await auth();
    const r = await context.request.post(`${base}/api/projects/register`, {
      headers: { Origin: base, "X-Factory-CSRF": csrf },
      data: { path: path.join(fixture, name), trust_project: true },
    });
    assert.ok(r.ok(), await r.text());
    const id = (await r.json()).project_id;
    set(id);
    await mutate(id, "open");
  }
  if (readOnlyQualification)
    await mutate(a, "recipes", {
      id: "p07-analysis",
      name: "P07 read-only research",
      roles: ["planner", "worker", "reviewer", "researcher"],
      planner_guidance:
        "For explicitly read-only analysis objectives create ONE researcher advisory task. Source is supplied; no file writes, shell tools or publication. Never replace a development objective with analysis.",
    });
  result.projects = { a, b, c, d };
  page = await context.newPage();
  page.on("pageerror", (error) => result.errors.push(error.message));
  await page.goto(`${base}/workspace.html`);
  await select(a);
  await test("CHAT-01 real project conversation and safe empty state", async () => {
    conversation = await newConversation();
    assert.match(conversation, /^c_/);
    const meta = await get(a, `conversations/${conversation}`);
    assert.equal(meta.project_id, a);
    await shot("empty-dark");
  });
  await test(
    controlledQualification
      ? "CHAT-02 controlled configured CLI response; no model qualification"
      : "CHAT-02 genuine configured Codex question response",
    async () => {
      const time = performance.now();
      await send(
        "Explain what source.py's add function does. This is a question, do not create a mission or run commands.",
      );
      await shot("processing");
      const response = await waitResponse(a, conversation);
      assert.equal(response.status, "finished", response.content);
      assert.equal(response.provenance.source, "agent");
      assert.ok(response.provenance.agent_session_id);
      assert.equal(
        (await get(a, `conversations/${conversation}/missions`)).length,
        0,
      );
      result.measurements.push({
        metric: "real_agent_response",
        ms: performance.now() - time,
      });
      await eventually(
        async () =>
          await panel()
            .locator(".chat-message[data-role=orchestrator]")
            .last()
            .innerText()
            .then((text) => text.includes(response.content.slice(0, 35))),
        "rendered response",
      );
      await shot("real-conversation");
    },
  );
  await test("CHAT-25 saved file context uses authorized P05 identities", async () => {
    await panel().getByText("Attach project context", { exact: true }).click();
    await panel().getByLabel("Repository-relative path").fill("source.py");
    await panel()
      .getByRole("button", { name: "Attach saved file", exact: true })
      .click();
    await panel().locator(".chat-reference").waitFor();
    await send(
      "Explain the supplied add function in one concise sentence. Do not make changes.",
    );
    const response = await waitResponse(a, conversation);
    assert.equal(response.status, "finished", response.content);
    const messages = await get(a, `conversations/${conversation}/messages`);
    const user = messages.find((m) => m.role === "user");
    assert.equal(user.provenance.context[0].file.project_id, a);
    assert.ok(user.provenance.context[0].content.includes("return a + b"));
  });
  let realRun;
  await test(
    controlledQualification
      ? "CHAT-07/08 controlled process plans and executes a real filesystem workflow"
      : "CHAT-07/08 real model plans and executes a mission automatically",
    async () => {
      await send(
        controlledQualification
          ? "START_MISSION"
          : readOnlyQualification
            ? "Create and execute a read-only research mission using the p07-analysis recipe. Analyze this supplied function: def add(a,b): return a+b. Have one researcher advisory task explain behavior, edge cases and two suggested tests. The output must be a persisted research artifact. Do not use shell tools, modify files, run tests or publish; this is a non-mutating analysis objective."
            : "Create a tiny Python arithmetic module arithmetic.py with multiply(a,b), and a unittest test_arithmetic.py checking multiply(6,7)==42 and a negative input. Run the unittest using installed python. Keep this to ONE worker implementation task; the engine supplies its own final review. Do not publish, push, install packages or change unrelated files.",
      );
      await eventually(
        async () => {
          const missions = await get(
            a,
            `conversations/${conversation}/missions`,
          );
          if (!missions.length) return false;
          realRun = missions[0].run_id;
          return true;
        },
        "mission creation",
        180000,
      );
      await shot("mission-planning");
      await eventually(
        async () => {
          const mission = (
            await get(a, `conversations/${conversation}/missions`)
          )[0];
          return mission.status === "active" || mission.terminal;
        },
        "automatic execution",
        240000,
      );
      await shot("mission-running");
      const response = await waitResponse(a, conversation, 600000);
      assert.equal(response.status, "finished", response.content);
      const mission = (
        await get(a, `conversations/${conversation}/missions`)
      )[0];
      if (controlledQualification) result.controlledMission = mission;
      else result.realMission = mission;
      assert.equal(mission.status, "completed", JSON.stringify(mission));
      assert.ok(mission.completed > 0);
      if (!readOnlyQualification) assert.ok(mission.integration_sha);
      assert.ok(mission.sessions.some((s) => s.role === "planner"));
      if (readOnlyQualification) {
        assert.ok(mission.sessions.some((s) => s.role === "researcher"));
        assert.ok(mission.artifacts.length > 0);
        assert.deepEqual(mission.changed_files, []);
      } else {
        assert.ok(mission.sessions.some((s) => s.role === "worker"));
        assert.ok(
          mission.changed_files.includes(
            controlledQualification ? "proof.txt" : "arithmetic.py",
          ),
        );
        if (controlledQualification)
          assert.ok(
            run(
              "git",
              ["show", `${mission.integration_sha}:proof.txt`],
              path.join(fixture, "Project A"),
            ).includes(fixture),
          );
      }
      await shot("mission-completed");
    },
  );
  await test("CHAT-15/16 verified progress and review entry", async () => {
    await view("Progress");
    const progress = page.locator(".chat-progress:visible").first();
    await progress
      .getByText("Tasks, agents and evidence", { exact: true })
      .first()
      .click();
    await progress
      .getByText("Actual agent sessions", { exact: true })
      .waitFor();
    await shot("progress-details");
    await progress
      .getByRole("button", { name: "Open workflow and review" })
      .first()
      .click();
    await page
      .locator(`.ws-view:visible[data-project-id="${a}"] .ws-state-line`)
      .getByText(`#${realRun}`, { exact: true })
      .first()
      .waitFor();
    await view("Orchestrator");
  });
  await test("CHAT inspection reads an existing project mission without taking run ownership", async () => {
    const inspection = await newConversation();
    await send(
      controlledQualification
        ? "INSPECT_RUN_1"
        : `Use inspect_mission to inspect existing project workflow #${realRun}; do not create or execute a new mission.`,
    );
    const response = await waitResponse(a, inspection);
    assert.equal(response.status, "finished", response.content);
    assert.equal(response.provenance.snapshot.run_id, realRun);
    assert.equal(response.run_id, null);
    assert.equal(
      (await get(a, `conversations/${inspection}/missions`)).length,
      0,
    );
    await panel()
      .getByRole("button", { name: `Workflow #${realRun}`, exact: true })
      .click();
    await page
      .locator(`.ws-view:visible[data-project-id="${a}"] .ws-state-line`)
      .getByText(`#${realRun}`, { exact: true })
      .first()
      .waitFor();
    await view("Orchestrator");
    await shot("existing-mission-reference");
    await panel()
      .getByLabel("Conversation", { exact: true })
      .selectOption(conversation);
  });
  await test("CHAT-03/04 browser reload and backend restart retain history", async () => {
    const before = await get(a, `conversations/${conversation}/messages`);
    await page.reload();
    await panel().getByLabel("Message to orchestrator").waitFor();
    assert.equal(
      await panel().getByLabel("Conversation", { exact: true }).inputValue(),
      conversation,
    );
    await stop();
    await start();
    await auth();
    await mutate(a, "open");
    await page.reload();
    await eventually(async () => {
      const values = await get(a, `conversations/${conversation}/messages`);
      return values.length === before.length && values[0].id === before[0].id;
    }, "restart history");
    await shot("restored");
  });
  await test("CHAT-05/24 project switching retains scoped conversations", async () => {
    await mutate(b, "open");
    await select(b);
    const cb = await newConversation();
    await send("Answer briefly: what is 6 times 7? Do not create a mission.");
    await select(a);
    const before = await panel()
      .getByLabel("Conversation", { exact: true })
      .inputValue();
    assert.equal(before, conversation);
    await waitResponse(b, cb);
    assert.equal(
      await panel().getByLabel("Conversation", { exact: true }).inputValue(),
      conversation,
    );
    const foreign = await context.request.get(
      `${base}/api/projects/${b}/conversations/${conversation}/messages`,
    );
    assert.equal(foreign.status(), 404);
    await shot("multi-project");
  });
  await mutate(c, "open");
  await select(c);
  let controlledConversation = await newConversation();
  await test("CHAT-12 form launches a real controlled workflow", async () => {
    await panel()
      .getByRole("button", { name: "Mission form", exact: true })
      .click();
    await panel()
      .getByLabel("Mission objective")
      .fill("Controlled form mission");
    await panel()
      .getByLabel("Acceptance criteria, one per line")
      .fill("proof.txt exists");
    await shot("mission-form");
    await submitClick(
      panel().getByRole("button", {
        name: "Plan and start mission",
        exact: true,
      }),
    );
    const response = await waitResponse(c, controlledConversation);
    assert.equal(response.status, "finished", response.content);
    const missions = await get(
      c,
      `conversations/${controlledConversation}/missions`,
    );
    assert.equal(missions[0].status, "completed");
  });
  await test("CHAT-13/14 authored visual DAG is the executed plan", async () => {
    await panel()
      .getByRole("button", { name: "Visual plan", exact: true })
      .click();
    await panel()
      .getByLabel("Mission objective")
      .fill("Exact authored objective");
    await panel()
      .getByRole("button", { name: "Add task", exact: true })
      .click();
    await panel()
      .getByLabel("Title T1", { exact: true })
      .fill("Exact authored task");
    await panel()
      .getByLabel("Objective T1", { exact: true })
      .fill("Write project proof");
    await panel()
      .getByLabel("Acceptance T1", { exact: true })
      .fill("proof exists");
    await panel().getByLabel("Dependencies T1", { exact: true }).fill("T1");
    await panel()
      .getByRole("button", { name: "Validate and launch plan", exact: true })
      .click();
    await panel()
      .getByText("Dependencies contain a cycle or unknown task.", {
        exact: true,
      })
      .waitFor();
    await panel().getByLabel("Dependencies T1", { exact: true }).fill("");
    await shot("visual-plan");
    await submitClick(
      panel().getByRole("button", {
        name: "Validate and launch plan",
        exact: true,
      }),
    );
    await waitResponse(c, controlledConversation);
    const mission = (
      await get(c, `conversations/${controlledConversation}/missions`)
    )[0];
    assert.equal(mission.tasks[0].title, "Exact authored task");
    assert.equal(
      mission.sessions.filter((s) => s.role === "planner").length,
      0,
    );
  });
  await test("CHAT-17 controlled failure remains a failed workflow", async () => {
    await fs.writeFile(
      path.join(fixture, "Controlled C/.factory/config.toml"),
      configuration(false, 'P07_FAIL = "1"'),
    );
    await send("START_MISSION");
    await waitResponse(c, controlledConversation);
    assert.equal(
      (await get(c, `conversations/${controlledConversation}/missions`))[0]
        .status,
      "failed",
    );
    await shot("failure-state");
    await fs.writeFile(
      path.join(fixture, "Controlled C/.factory/config.toml"),
      configuration(false),
    );
  });
  await test("CHAT-26 unsaved CodeMirror selection is actual attached context", async () => {
    await view("Explorer");
    const row = page
      .locator(".files-explorer:visible")
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", { hasText: /^source\.py$/ }),
      })
      .first();
    await row.click();
    const editor = page
      .getByRole("textbox", { name: "Source editor source.py", exact: true })
      .last();
    await editor.fill(
      "def add(a,b):\n    return a + b + 7 # unsaved crab \u{1f980}\n",
    );
    await editor.press("Control+a");
    await view("Orchestrator");
    const details = panel().locator(".chat-context");
    if (!(await details.getAttribute("open")))
      await details.locator("summary").click();
    const editorOption = panel()
      .getByLabel("Open editor file")
      .locator("option")
      .filter({ hasText: "source.py" })
      .first();
    await editorOption.waitFor({ state: "attached" });
    const documentId = await editorOption.getAttribute("value");
    assert.ok(documentId);
    await panel().getByLabel("Open editor file").selectOption(documentId);
    await panel().getByLabel("Use editor selection when present").check();
    await panel()
      .getByRole("button", { name: "Attach editor context", exact: true })
      .click();
    await send("Explain the unsaved selection without editing any files.");
    await waitResponse(c, controlledConversation);
    const user = (
      await get(c, `conversations/${controlledConversation}/messages`)
    ).find((m) => m.role === "user");
    assert.equal(user.provenance.context[0].state, "unsaved");
    assert.ok(
      user.provenance.context[0].content.includes("unsaved crab \u{1f980}"),
    );
    assert.ok(
      !(
        await fs.readFile(path.join(fixture, "Controlled C/source.py"), "utf8")
      ).includes("unsaved crab"),
    );
    await shot("unsaved-context");
  });
  await test("CHAT-18/19 switching and disconnecting do not cancel; explicit cancel is scoped", async () => {
    const gateC = path.join(fixture, "gate-c"),
      gateB = path.join(fixture, "gate-b");
    await fs.writeFile(
      path.join(fixture, "Controlled C/.factory/config.toml"),
      configuration(false, `P07_WAIT = ${JSON.stringify(gateC)}`),
    );
    const accepted = await send("START_MISSION");
    await eventually(async () => {
      const missions = await get(
        c,
        `conversations/${controlledConversation}/missions`,
      );
      return missions[0]?.sessions.some(
        (s) => s.role === "worker" && s.status === "running",
      );
    }, "controlled C worker");
    await fs.writeFile(
      path.join(fixture, "Project B/.factory/config.toml"),
      configuration(false, `P07_WAIT = ${JSON.stringify(gateB)}`),
    );
    await select(b);
    const cb = await newConversation();
    await send("START_MISSION");
    await eventually(async () => {
      const missions = await get(b, `conversations/${cb}/missions`);
      return missions[0]?.sessions.some(
        (s) => s.role === "worker" && s.status === "running",
      );
    }, "independent B worker");
    await page.reload();
    assert.equal(
      (await get(c, `conversations/${controlledConversation}/missions`))[0]
        .status,
      "active",
    );
    await mutate(
      c,
      `conversations/${controlledConversation}/commands/${accepted.id}/cancel`,
    );
    await eventually(
      async () =>
        (await get(c, `conversations/${controlledConversation}/missions`))[0]
          .status === "cancelled",
      "scoped cancellation",
    );
    assert.equal(
      (await get(b, `conversations/${cb}/missions`))[0].status,
      "active",
    );
    await fs.writeFile(gateB, "release");
    await waitResponse(b, cb);
    assert.equal(
      (await get(b, `conversations/${cb}/missions`))[0].status,
      "completed",
    );
    await fs.writeFile(
      path.join(fixture, "Controlled C/.factory/config.toml"),
      configuration(false),
    );
    await select(c);
    await view("Orchestrator");
    await shot("cancelled");
  });
  await test("CHAT-23 two browser windows converge on durable ordered history", async () => {
    const primary = page,
      second = await context.newPage();
    page = second;
    await page.goto(`${base}/workspace.html`);
    await select(c);
    await view("Orchestrator");
    await panel()
      .getByLabel("Conversation", { exact: true })
      .selectOption(controlledConversation);
    await send("Question from a second browser window.");
    await waitResponse(c, controlledConversation);
    const count = (
      await get(c, `conversations/${controlledConversation}/messages`)
    ).length;
    await eventually(
      async () =>
        (await primary.locator(".chat-panel:visible .chat-message").count()) ===
        count,
      "primary event convergence",
    );
    await shot("two-windows");
    await second.close();
    page = primary;
  });
  await test("CHAT-06 duplicate API submission resolves to one command", async () => {
    const cid = (await mutate(c, "conversations", { title: "Duplicate test" }))
      .id;
    const body = {
      key: "same-key",
      workspace_session_id: `s_${"a".repeat(32)}`,
      content: "Question",
      context: [],
    };
    const first = await mutate(c, `conversations/${cid}/messages`, body),
      again = await mutate(c, `conversations/${cid}/messages`, body);
    assert.equal(first.id, again.id);
    await waitResponse(c, cid);
    assert.equal((await get(c, `conversations/${cid}/messages`)).length, 2);
  });
  await test("CHAT-20/21 ordered durable event replay", async () => {
    const response = await context.request.get(
      `${base}/api/projects/${c}/conversations/${controlledConversation}/messages`,
    );
    assert.ok(response.ok());
    const controller = new AbortController();
    const csrf = await auth();
    const cookies = (await context.cookies())
      .map((c) => `${c.name}=${c.value}`)
      .join("; ");
    const stream = await fetch(
      `${base}/api/projects/${c}/conversations/${controlledConversation}/events?after=0`,
      {
        headers: { Origin: base, Cookie: cookies, "X-Factory-CSRF": csrf },
        signal: controller.signal,
      },
    );
    const reader = stream.body.getReader();
    let text = "";
    for (let i = 0; i < 5; i++) {
      const chunk = await reader.read();
      text += new TextDecoder().decode(chunk.value);
      if (text.includes("conversation.response.completed")) break;
    }
    controller.abort();
    const ids = [...text.matchAll(/^id: (\d+)$/gm)].map((m) => Number(m[1]));
    assert.ok(ids.length > 0);
    assert.ok(
      ids.every((value, index) => index === 0 || value > ids[index - 1]),
    );
    result.events.push({
      project: c,
      conversation: controlledConversation,
      ids,
    });
  });
  await test("CHAT-27 browser authentication, CSRF and protected context", async () => {
    const route = `${base}/api/projects/${c}/conversations`;
    assert.ok(
      !(
        await context.request.post(route, {
          headers: { Origin: base },
          data: { title: "No CSRF" },
        })
      ).ok(),
    );
    const roots = await get(c, "files/roots");
    const csrf = await auth();
    const r = await context.request.post(
      `${route}/${controlledConversation}/messages`,
      {
        headers: { Origin: base, "X-Factory-CSRF": csrf },
        data: {
          key: "protected",
          workspace_session_id: `s_${"a".repeat(32)}`,
          content: "Read config",
          context: [
            {
              file: {
                project_id: c,
                root_id: roots.roots[0].id,
                path: ".factory/config.toml",
              },
              revision: "",
            },
          ],
        },
      },
    );
    assert.equal(r.status(), 403);
  });
  await test("CHAT-28 no-agent state retains text and configuration guidance", async () => {
    await mutate(d, "open");
    await select(d);
    await newConversation();
    await panel()
      .getByText("Configure an orchestrator", { exact: true })
      .waitFor();
    await shot("configuration-required");
  });
  await test("light, narrow desktop, keyboard and accessibility", async () => {
    await select(c);
    await view("Orchestrator");
    await panel()
      .getByLabel("Conversation", { exact: true })
      .selectOption(controlledConversation);
    await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    result.accessibility = { violations: [], themes: [] };
    for (const theme of ["dark", "light"]) {
      await page.getByLabel("Theme").selectOption(theme);
      await shot(theme);
      const audit = await page.evaluate(async () =>
        window.axe.run(document, {
          runOnly: { type: "tag", values: ["wcag2a", "wcag2aa", "wcag22aa"] },
        }),
      );
      result.accessibility.themes.push({
        theme,
        violations: audit.violations,
        incomplete: audit.incomplete.map((i) => ({
          id: i.id,
          impact: i.impact,
        })),
      });
      result.accessibility.violations.push(...audit.violations);
      assert.equal(
        audit.violations.length,
        0,
        JSON.stringify(
          audit.violations.map((v) => ({
            id: v.id,
            nodes: v.nodes.map((n) => n.target),
          })),
        ),
      );
    }
    await page.setViewportSize({ width: 1024, height: 768 });
    await page.keyboard.press("Alt+c");
    assert.ok(
      await panel()
        .getByLabel("Message to orchestrator")
        .evaluate((node) => node === document.activeElement),
    );
    await shot("narrow");
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    );
    await page.setViewportSize({ width: 1440, height: 900 });
  });
  await test("CHAT-30 legacy V1 routes remain operational", async () => {
    const legacy = await context.newPage();
    await legacy.goto(`${base}/?project=${c}`);
    await legacy.getByRole("table").waitFor();
    await legacy
      .getByRole("button", { name: "Agent Graph", exact: true })
      .click();
    await legacy.locator(".react-flow").waitFor();
    await legacy.getByRole("button", { name: "Settings", exact: true }).click();
    await legacy
      .getByRole("heading", { name: "Settings", exact: true })
      .waitFor();
    assert.equal(
      (await legacy.request.get(`${base}/api/projects/${c}/runs`)).status(),
      200,
    );
    await legacy.close();
  });
  result.browserHeap = await page.evaluate(
    () => performance.memory?.usedJSHeapSize ?? null,
  );
  assert.equal(result.errors.length, 0, JSON.stringify(result.errors));
} finally {
  await fs.writeFile(
    path.join(output, "browser-results.json"),
    JSON.stringify(result, null, 2),
  );
  await context?.close();
  await browser?.close();
  await stop();
}
