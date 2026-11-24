import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { createRequire } from "node:module";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output = path.join(root, ".factory/v2/p07-r1");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const resume =
  process.env.ASF_P07_R1_RESUME === "1"
    ? JSON.parse(
        await fs.readFile(path.join(output, "acceptance-results.json"), "utf8"),
      )
    : null;
try {
  await fs.access(path.join(output, "acceptance-results.json"));
  const archive = path.join(output, "attempts", String(Date.now()));
  await fs.mkdir(archive, { recursive: true });
  await fs.copyFile(
    path.join(output, "acceptance-results.json"),
    path.join(archive, "acceptance-results.json"),
  );
  await fs.cp(
    path.join(output, "screenshots"),
    path.join(archive, "screenshots"),
    { recursive: true },
  );
  await fs.copyFile(
    path.join(output, "acceptance-server.log"),
    path.join(archive, "acceptance-server.log"),
  );
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
const preflight = JSON.parse(
  await fs.readFile(
    process.env.ASF_P07_R1_PREFLIGHT ??
      path.join(output, "mxc-no-login-preflight.json"),
    "utf8",
  ),
);
assert.equal(
  preflight.write,
  "PASS",
  "A real model write/read/test preflight is required first",
);
const binary =
  process.env.ASF_P07_R1_BINARY ?? path.join(output, "run/factory-r1.exe");
const cli =
  process.env.ASF_P07_R1_CODEX ??
  "codex";
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const fixture =
  resume?.fixture ??
  (await fs.mkdtemp(path.join(os.tmpdir(), "asf-p07-r1-acceptance-")));
const registry = path.join(fixture, "registry.sqlite3");
const result = {
  schema: 1,
  started: new Date().toISOString(),
  fixture,
  registry,
  binary,
  binarySha256: hash(await fs.readFile(binary)),
  scriptSha256: hash(await fs.readFile(fileURLToPath(import.meta.url))),
  cli,
  tests: [],
  pathways: {},
  screenshots: [],
  errors: [],
  guarantees: [
    "Real configured model for every Worker and Reviewer",
    "No application patch is generated or applied by this harness",
    "Source repository main trees are not checked out to integration branches",
  ],
};
if (resume) {
  assert.equal(
    resume.binarySha256,
    result.binarySha256,
    "Resume requires the identical qualified binary",
  );
  result.pathways = resume.pathways;
  result.tests = resume.tests.filter((test) => test.status === "passed");
  result.screenshots = resume.screenshots;
  result.resumedFrom = {
    started: resume.started,
    scriptSha256: resume.scriptSha256,
    failure: resume.failure,
  };
}
function run(program, args, cwd) {
  const started = performance.now();
  const r = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
    timeout: 60000,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  return {
    argv: [program, ...args],
    cwd,
    exit: r.status,
    stdout: r.stdout ?? "",
    stderr: r.stderr ?? "",
    error: r.error?.message,
    ms: performance.now() - started,
  };
}
function ok(command) {
  assert.equal(command.exit, 0, JSON.stringify(command));
  return command.stdout.trim();
}
result.sourceCommit = ok(run("git", ["rev-parse", "HEAD"], root));
result.cliVersion = ok(run(cli, ["--version"], root));
function profiles() {
  const agents = ["planner", "worker", "reviewer"]
    .map((role) => {
      const args = [
        "exec",
        "--ignore-user-config",
        "--ephemeral",
        "--disable",
        "apps",
        "--sandbox",
        role === "worker" ? "workspace-write" : "read-only",
        "-c",
        "windows.sandbox=mxc",
        "-c",
        "allow_login_shell=false",
        "--color",
        "never",
      ];
      return `[agents.${role}]\nkind = "codex"\ncommand = ${JSON.stringify(cli)}\nargs = ${JSON.stringify(args)}\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE = "1"\n[[role_assignments]]\nrole = "${role}"\nagent = "${role}"\npreferred = true\n`;
    })
    .join("\n");
  return (
    agents +
    ["planner", "worker", "reviewer"]
      .map(
        (role) => `
[policies.roles.${role}.filesystem]
read = ["**"]
write = ${role === "worker" ? '["src/**", "tests/**"]' : "[]"}
[policies.roles.${role}.commands]
mode = "restricted"
allow = ["python", "python3", "git", "rg", "Get-Content", "Set-Content", "Get-ChildItem", "Get-Location", "Test-Path", "powershell", "powershell.exe"]
deny = ["curl", "Invoke-WebRequest", "cmd", "bash", "sh"]
[policies.roles.${role}.network]
mode = "deny"
`,
      )
      .join("\n")
  );
}
const objective =
  "Inspect src/calculator.py and its failing regression test. Correct ceil_div so it returns the mathematical ceiling of an integer value divided by a positive integer divisor, including exact multiples, zero and negative values. Preserve existing positive non-exact behavior and ValueError for non-positive divisors. Add meaningful regression tests and run python -m unittest discover -s tests -v. Use one Worker implementation task; ASF supplies its independent final Reviewer. Change only src/calculator.py and tests/test_calculator.py. Do not commit, stage, publish, push, install packages, change configuration or request broader permissions. Execute individual commands without semicolon or pipe bundles. In the commands report list exact executable invocations separately; put descriptive results in the results field. ASF will commit and integrate approved changes.";
for (const name of ["Chat", "Form", "Visual"]) {
  if (resume) continue;
  const folder = path.join(fixture, name);
  await fs.mkdir(path.join(folder, "src"), { recursive: true });
  await fs.mkdir(path.join(folder, "tests"));
  await fs.writeFile(
    path.join(folder, ".gitignore"),
    ".factory/\n__pycache__/\n*.pyc\n",
  );
  await fs.writeFile(
    path.join(folder, "README.md"),
    `Disposable ASF R1 ${name} calculator. No remote or publication.\n`,
  );
  await fs.writeFile(path.join(folder, "src/__init__.py"), "");
  const source = `PROJECT = ${JSON.stringify(name)}\n\ndef ceil_div(value, divisor):\n    if divisor <= 0:\n        raise ValueError("divisor must be positive")\n    return value // divisor + 1\n`;
  const tests = `import unittest\nfrom src.calculator import ceil_div\n\nclass CalculatorTests(unittest.TestCase):\n    def test_positive_non_exact(self):\n        self.assertEqual(ceil_div(7, 3), 3)\n\n    def test_exact_multiple(self):\n        self.assertEqual(ceil_div(6, 3), 2)\n\n    def test_invalid_divisor(self):\n        for divisor in [0, -1]:\n            with self.assertRaises(ValueError):\n                ceil_div(7, divisor)\n`;
  await fs.writeFile(path.join(folder, "src/calculator.py"), source);
  await fs.writeFile(path.join(folder, "tests/test_calculator.py"), tests);
  ok(run("git", ["init", "-q", "-b", "main"], folder));
  ok(run("git", ["config", "user.name", "ASF R1"], folder));
  ok(run("git", ["config", "user.email", "r1@example.invalid"], folder));
  ok(run("git", ["add", "."], folder));
  ok(
    run(
      "git",
      ["commit", "-qm", "reproduce exact-multiple calculator defect"],
      folder,
    ),
  );
  ok(run(binary, ["init"], folder));
  await fs.writeFile(path.join(folder, ".factory/config.toml"), profiles());
  const baseline = run(
    "python",
    ["-m", "unittest", "discover", "-s", "tests", "-v"],
    folder,
  );
  assert.notEqual(baseline.exit, 0);
  assert.match(baseline.stderr, /FAIL: test_exact_multiple/);
  assert.match(baseline.stderr, /test_positive_non_exact.*ok/);
  assert.match(baseline.stderr, /test_invalid_divisor.*ok/);
  assert.equal(ok(run("git", ["remote"], folder)), "");
  result.pathways[name] = {
    root: folder,
    baseline,
    initialSource: source,
    initialTests: tests,
    initialHead: ok(run("git", ["rev-parse", "HEAD"], folder)),
    submittedObjective: objective,
  };
}
const port = await new Promise((resolve) => {
  const socket = net.createServer();
  socket.listen(0, "127.0.0.1", () => {
    const n = socket.address().port;
    socket.close(() => resolve(n));
  });
});
const base = `http://127.0.0.1:${port}`;
const log = await fs.open(path.join(output, "acceptance-server.log"), "w");
const child = spawn(
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
    cwd: result.pathways.Chat.root,
    windowsHide: true,
    stdio: ["ignore", log.fd, log.fd],
  },
);
await log.close();
let browser, context, page;
async function persist() {
  await fs.writeFile(
    path.join(output, "acceptance-results.json"),
    JSON.stringify(result, null, 2),
  );
}
async function eventually(fn, label, timeout = 40000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Timed out: ${label}`);
}
async function auth() {
  const r = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(r.ok(), await r.text());
  return (await r.json()).csrfToken;
}
async function get(project, suffix) {
  const r = await context.request.get(
    `${base}/api/projects/${project}/${suffix}`,
  );
  assert.ok(r.ok(), await r.text());
  return r.json();
}
async function mutate(url, data, method = "POST", expected = 200) {
  const r = await context.request.fetch(base + url, {
    method,
    headers: { Origin: base, "X-Factory-CSRF": await auth() },
    data,
  });
  assert.equal(r.status(), expected, await r.text());
  return r.json();
}
async function test(name, fn) {
  const start = performance.now();
  try {
    await fn();
    result.tests.push({
      name,
      status: "passed",
      ms: performance.now() - start,
    });
    console.log(`PASS ${name}`);
  } catch (e) {
    result.tests.push({ name, status: "failed", error: String(e) });
    throw e;
  } finally {
    await persist();
  }
}
async function shot(name) {
  const file = `${name}.png`;
  await page.screenshot({ path: path.join(output, "screenshots", file) });
  result.screenshots.push(file);
}
async function view(name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
const panel = () => page.locator(".chat-panel:visible").first();
async function newConversation() {
  await view("Orchestrator");
  const pending = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" &&
      /\/conversations$/.test(new URL(r.url()).pathname),
  );
  await panel()
    .getByRole("button", { name: "New conversation", exact: true })
    .click();
  const r = await pending;
  assert.ok(r.ok(), await r.text());
  const conversation = (await r.json()).id;
  await eventually(
    async () =>
      (await panel()
        .getByLabel("Conversation", { exact: true })
        .inputValue()) === conversation,
    "new conversation selection",
  );
  return conversation;
}
async function submit(name, item) {
  item.submittedObjective = objective;
  await page.getByLabel("Open registered project").selectOption(item.project);
  item.conversation = await newConversation();
  const pending = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" &&
      /\/messages$/.test(new URL(r.url()).pathname),
  );
  if (name === "Chat") {
    await panel().getByLabel("Message to orchestrator").fill(objective);
    await panel().getByRole("button", { name: "Send", exact: true }).click();
  } else {
    await panel()
      .getByRole("button", {
        name: name === "Form" ? "Mission form" : "Visual plan",
        exact: true,
      })
      .click();
    await panel().getByLabel("Mission objective").fill(objective);
    await panel()
      .getByLabel("Acceptance criteria, one per line")
      .fill(
        "A meaningful implementation fix and regression tests\nAll unittest cases pass; no unrelated file changes",
      );
    if (name === "Visual") {
      await panel()
        .getByRole("button", { name: "Add task", exact: true })
        .click();
      await panel()
        .getByLabel("Title T1", { exact: true })
        .fill("Fix ceiling division and verify regressions");
      await panel().getByLabel("Objective T1", { exact: true }).fill(objective);
      await panel()
        .getByLabel("Acceptance T1", { exact: true })
        .fill(
          "Exact multiples, zero, negative and positive values are correct\nRun python -m unittest discover -s tests -v\nOnly calculator implementation and tests change",
        );
    }
    await shot(`${name.toLowerCase()}-entry`);
    await panel()
      .getByRole("button", {
        name:
          name === "Form"
            ? "Plan and start mission"
            : "Validate and launch plan",
        exact: true,
      })
      .click();
  }
  const accepted = await pending;
  assert.equal(accepted.status(), 202, await accepted.text());
  item.command = await accepted.json();
  item.mission = await eventually(
    async () =>
      (
        await get(item.project, `conversations/${item.conversation}/missions`)
      )[0],
    "mission association",
    180000,
  );
  await view("Progress");
  await shot(`${name.toLowerCase()}-planning`);
}
async function runMission(name, item) {
  if (item.mission?.status !== "completed") {
    delete item.manualWorktreeWrite;
    delete item.runningCapture;
    await submit(name, item);
    await eventually(
      async () => {
        item.mission = (
          await get(item.project, `conversations/${item.conversation}/missions`)
        )[0];
        if (item.mission.terminal) return true;
        if (
          item.mission.sessions.some(
            (s) => s.role === "worker" && s.status === "running",
          )
        ) {
          const roots = await get(item.project, "files/roots");
          const managed = roots.roots.find(
            (r) =>
              r.kind === "task" ||
              r.kind === "agent_worktree" ||
              r.kind === "task_worktree",
          );
          if (managed && !item.manualWorktreeWrite) {
            const snapshot = await get(
              item.project,
              `files/content?root_id=${managed.id}&path=src/calculator.py`,
            );
            assert.equal(snapshot.read_only, true);
            item.manualWorktreeWrite = await mutate(
              `/api/projects/${item.project}/files/content`,
              {
                file: snapshot.file,
                expected_revision: snapshot.revision,
                content: snapshot.content,
              },
              "PUT",
              403,
            );
          }
          if (!item.runningCapture) {
            await shot(`${name.toLowerCase()}-worker`);
            item.runningCapture = true;
          }
        }
        return false;
      },
      "real mission completion",
      480000,
    );
  } else {
    await page.getByLabel("Open registered project").selectOption(item.project);
    await view("Orchestrator");
    await panel()
      .getByLabel("Conversation", { exact: true })
      .selectOption(item.conversation);
    await view("Progress");
  }
  assert.equal(item.mission.status, "completed", JSON.stringify(item.mission));
  assert.equal(
    item.manualWorktreeWrite?.code,
    "worktree_read_only",
    "Managed worktree must reject ordinary editor saves while its real Worker can write",
  );
  assert.ok(item.mission.integration_sha);
  assert.ok(item.mission.changed_files.includes("src/calculator.py"));
  assert.ok(item.mission.changed_files.includes("tests/test_calculator.py"));
  assert.ok(
    item.mission.changed_files.every((f) =>
      ["src/calculator.py", "tests/test_calculator.py"].includes(f),
    ),
  );
  item.detail = await get(item.project, `runs/${item.mission.run_id}`);
  const attempts = item.detail.attempts;
  const approved = attempts.filter((a) => a.status === "approved");
  assert.ok(approved.length);
  assert.ok(
    approved.every(
      (a) => a.review?.decision === "approve" && a.evidence?.diffPatch?.trim(),
    ),
  );
  item.patch = ok(
    run(
      "git",
      [
        "diff",
        item.initialHead,
        item.mission.integration_sha,
        "--",
        "src/calculator.py",
        "tests/test_calculator.py",
      ],
      item.root,
    ),
  );
  assert.match(item.patch, /[+-].*return/);
  const reviewer = item.detail.sessions.find(
    (s) =>
      s.role === "reviewer" &&
      s.status === "success" &&
      s.attemptId === approved.at(-1).id,
  );
  assert.ok(reviewer);
  item.reviewSessionId = reviewer.id;
  assert.ok(
    reviewer.stderr.includes(
      approved.at(-1).evidence.diffPatch.trim().slice(0, 100),
    ),
    "Reviewer must receive the captured patch",
  );
  assert.equal(
    ok(run("git", ["rev-parse", "main"], item.root)),
    item.initialHead,
  );
  assert.equal(
    await fs.readFile(path.join(item.root, "src/calculator.py"), "utf8"),
    item.initialSource,
  );
  assert.equal(
    ok(
      run(
        "git",
        ["rev-parse", `factory/run-${item.mission.run_id}`],
        item.root,
      ),
    ),
    item.mission.integration_sha,
  );
  const verify =
    item.verificationRoot ??
    path.join(fixture, `${name} integrated verification`);
  if (!item.verificationRoot) {
    ok(
      run(
        "git",
        [
          "clone",
          "-q",
          "--no-hardlinks",
          "--no-checkout",
          "--",
          item.root,
          verify,
        ],
        fixture,
      ),
    );
    ok(
      run(
        "git",
        ["checkout", "--detach", item.mission.integration_sha],
        verify,
      ),
    );
    ok(run("git", ["remote", "remove", "origin"], verify));
  } else
    assert.equal(
      ok(run("git", ["rev-parse", "HEAD"], verify)),
      item.mission.integration_sha,
    );
  item.independentTests = run(
    "python",
    ["-m", "unittest", "discover", "-s", "tests", "-v"],
    verify,
  );
  ok(item.independentTests);
  item.independentProperties = run(
    "python",
    [
      "-c",
      `from src.calculator import ceil_div, PROJECT; from fractions import Fraction; from math import ceil; assert PROJECT == ${JSON.stringify(name)}; cases=[(n,d) for n in range(-100,101) for d in range(1,18)]+[(10**30,7),(-10**30,7)]; assert all(ceil_div(n,d)==ceil(Fraction(n,d)) for n,d in cases); print(str(len(cases))+' independent integer cases passed')`,
    ],
    verify,
  );
  ok(item.independentProperties);
  item.verifiedSource = await fs.readFile(
    path.join(verify, "src/calculator.py"),
    "utf8",
  );
  item.integrationBranch = `factory/run-${item.mission.run_id}`;
  item.verificationRoot = verify;
  item.testExecutionTrace = item.detail.sessions
    .filter((s) => s.role === "worker" || s.role === "reviewer")
    .map((s) => ({
      id: s.id,
      role: s.role,
      status: s.status,
      exit_code: s.exitCode,
      policy_audit: s.policyAudit,
      stdout: s.stdout,
      stderr: s.stderr,
      duration_ms: s.durationMs,
    }));
  assert.ok(
    item.testExecutionTrace.some(
      (s) => s.stderr.includes("unittest") && /Ran \d+ tests?/.test(s.stderr),
    ),
    "Actual CLI test execution trace required",
  );
  item.messages = await eventually(async () => {
    const rows = await get(
      item.project,
      `conversations/${item.conversation}/messages`,
    );
    return rows.some(
      (m) => m.id === item.command.response_id && m.status === "finished",
    )
      ? rows
      : false;
  }, "durable conversation result");
  assert.match(
    item.messages.find((m) => m.id === item.command.response_id).content,
    /is completed/,
  );
  await panel()
    .getByText(`Workflow #${item.mission.run_id} is completed.`, {
      exact: false,
    })
    .waitFor();
  await shot(`${name.toLowerCase()}-completed`);
  await page
    .getByRole("button", { name: "Open workflow and review", exact: true })
    .last()
    .click();
  const captured = page
    .locator(".ws-attempt-evidence:visible")
    .filter({ has: page.locator("summary", { hasText: / · approved$/ }) })
    .last();
  await captured.locator("summary").click();
  await captured.getByLabel(/Captured diff for task/).waitFor();
  await captured.getByLabel(/Captured diff for task/).scrollIntoViewIfNeeded();
  assert.match(await captured.innerText(), /Reviewer: approve/);
  assert.match(await captured.innerText(), /[+-].*return/);
  await shot(`${name.toLowerCase()}-captured-diff-review`);
  if (name === "Visual") {
    assert.equal(
      item.mission.tasks[0].title,
      "Fix ceiling division and verify regressions",
    );
    assert.equal(
      item.mission.sessions.filter((s) => s.role === "planner").length,
      0,
    );
  } else
    assert.ok(
      item.mission.sessions.some(
        (s) => s.role === "planner" && s.status === "success",
      ),
    );
  await fs.writeFile(
    path.join(output, `${name.toLowerCase()}-integrated.patch`),
    item.patch,
  );
  await persist();
}
try {
  await eventually(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  }, "server health");
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  result.browser = browser.version();
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
    colorScheme: "dark",
  });
  await auth();
  const registered = await (
    await context.request.get(`${base}/api/projects`)
  ).json();
  const canonical = (value) =>
    value
      .replace(/^\\\\\?\\/, "")
      .replaceAll("\\", "/")
      .toLowerCase();
  for (const name of ["Chat", "Form", "Visual"]) {
    const item = result.pathways[name];
    const owning = registered.find(
      (entry) => canonical(entry.canonical_root) === canonical(item.root),
    );
    if (!owning) continue;
    if (item.project && item.project !== owning.project_id) {
      result.repairedHarnessMetadata ??= [];
      result.repairedHarnessMetadata.push({
        pathway: name,
        previous: item.project,
        authoritative: owning.project_id,
        reason:
          "Resume must use the registry root identity, never listing position",
      });
    }
    item.project = owning.project_id;
  }
  for (const name of ["Form", "Visual"]) {
    const item = result.pathways[name];
    const entry = item.project
      ? { project_id: item.project }
      : await mutate(
          "/api/projects/register",
          {
            path: item.root,
            trust_project: true,
          },
          "POST",
          201,
        );
    item.project = entry.project_id;
    await mutate(`/api/projects/${item.project}/open`, {});
  }
  page = await context.newPage();
  page.on("pageerror", (error) => result.errors.push(error.message));
  await page.goto(`${base}/workspace.html`);
  for (const name of ["Chat", "Form", "Visual"]) {
    if (
      result.tests.some(
        (test) =>
          test.name ===
            `Real ${name.toLowerCase()} coding mission: defect, model diff, tests, review and integration` &&
          test.status === "passed",
      )
    )
      continue;
    await test(`Real ${name.toLowerCase()} coding mission: defect, model diff, tests, review and integration`, () =>
      runMission(name, result.pathways[name]));
  }
  await test("Persisted missions and conversations resolve to three distinct authoritative project roots", async () => {
    assert.equal(
      new Set(Object.values(result.pathways).map((item) => item.project)).size,
      3,
    );
    for (const item of Object.values(result.pathways)) {
      const missions = await get(
        item.project,
        `conversations/${item.conversation}/missions`,
      );
      const mission = missions.find(
        (mission) => mission.run_id === item.mission.run_id,
      );
      assert.equal(mission?.integration_sha, item.mission.integration_sha);
      item.detail = await get(item.project, `runs/${mission.run_id}`);
      item.reviewSessionId = item.detail.sessions.find(
        (session) =>
          session.role === "reviewer" &&
          item.detail.attempts.some(
            (attempt) =>
              attempt.id === session.attemptId && attempt.status === "approved",
          ),
      )?.id;
      assert.ok(item.reviewSessionId);
    }
  });
  await test("CodeMirror displays independently verified integration checkout, not unchanged main", async () => {
    const item = result.pathways.Chat;
    if (!item.verificationProject)
      ok(run(binary, ["init"], item.verificationRoot));
    const entry = item.verificationProject
      ? { project_id: item.verificationProject }
      : await mutate(
          "/api/projects/register",
          {
            path: item.verificationRoot,
            trust_project: true,
          },
          "POST",
          201,
        );
    item.verificationProject = entry.project_id;
    await mutate(`/api/projects/${entry.project_id}/open`, {});
    await page
      .getByLabel("Open registered project")
      .selectOption(entry.project_id);
    await view("Explorer");
    const explorer = page.locator(".files-explorer:visible").first();
    const sourceDirectory = explorer
      .getByRole("treeitem")
      .filter({ has: page.locator(".files-tree-name", { hasText: /^src$/ }) });
    if ((await sourceDirectory.getAttribute("aria-expanded")) !== "true")
      await sourceDirectory.click();
    await explorer
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", { hasText: /^calculator.py$/ }),
      })
      .click();
    const editor = page
      .getByRole("textbox", {
        name: "Source editor src/calculator.py",
        exact: true,
      })
      .first();
    await editor.waitFor();
    assert.equal(
      (await editor.locator(".cm-line").allTextContents()).join("\n").trim(),
      item.verifiedSource.replaceAll("\r\n", "\n").trim(),
    );
    await shot("verified-integration-codemirror");
  });
  assert.deepEqual(result.errors, []);
  result.finished = new Date().toISOString();
} catch (error) {
  result.failure = String(error);
  if (page) {
    await page.screenshot({
      path: path.join(output, "screenshots/acceptance-failure.png"),
    });
    result.screenshots.push("acceptance-failure.png");
  }
  throw error;
} finally {
  await persist();
  await context?.close();
  await browser?.close();
  if (child.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
}
