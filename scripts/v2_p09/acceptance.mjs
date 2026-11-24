import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const output =
  process.env.ASF_P09_OUTPUT ?? path.join(root, ".factory/v2/p09/acceptance");
const binary = process.env.ASF_P09_BINARY;
assert.ok(binary, "ASF_P09_BINARY must select the actual P09 executable");
const cli = process.env.ASF_P09_CODEX ?? "codex";
const native = process.argv.includes("--native");
const inspect = process.argv.includes("--inspect");
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const hash = (value) => createHash("sha256").update(value).digest("hex");
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const recordFile = path.join(output, "results.json");
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
let result;
try {
  result = JSON.parse(await fs.readFile(recordFile, "utf8"));
} catch (error) {
  if (error.code !== "ENOENT") throw error;
}
if (result && !native && !inspect)
  throw new Error("Setup evidence already exists; select a new output");
result ??= {
  schema: 1,
  started: new Date().toISOString(),
  fixture: await fs.mkdtemp(path.join(os.tmpdir(), "asf-p09-")),
  projects: {},
  profiles: {},
  screenshots: [],
  checks: [],
  measurements: [],
  pageErrors: [],
  machine: {
    platform: os.platform(),
    release: os.release(),
    cpu: os.cpus()[0].model,
    cpus: os.cpus().length,
    ram: os.totalmem(),
  },
};
const binaryHash = hash(await fs.readFile(binary));
if (result.binarySha256)
  assert.equal(
    result.binarySha256,
    binaryHash,
    "Resume must use the qualified executable",
  );
Object.assign(result, {
  binary,
  binarySha256: binaryHash,
  scriptSha256: hash(await fs.readFile(fileURLToPath(import.meta.url))),
  sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  }).stdout.trim(),
});
const persist = () =>
  fs.writeFile(recordFile, JSON.stringify(result, null, 2) + "\n");
result.scriptExecutions ??= [];
result.scriptExecutions.push({
  stage: native ? "native" : inspect ? "inspection" : "setup",
  sha256: result.scriptSha256,
  at: new Date().toISOString(),
});
function run(program, args, cwd, timeout = 60000) {
  const start = performance.now();
  const r = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    timeout,
    windowsHide: true,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" },
  });
  return {
    program,
    args,
    cwd,
    exit: r.status,
    stdout: r.stdout ?? "",
    stderr: r.stderr ?? "",
    error: r.error?.message,
    ms: performance.now() - start,
  };
}
function ok(r) {
  assert.equal(r.exit, 0, JSON.stringify(r));
  return r.stdout.trim();
}
async function eventually(fn, label, timeout = 60000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await delay(100);
  }
  throw new Error(`Timeout: ${label}`);
}
const source =
  "def clamp(value, lower, upper):\n    return min(max(value, lower), upper)\n";
const tests =
  "import unittest\nfrom src.math_tools import clamp\n\nclass ClampTests(unittest.TestCase):\n    def test_inside(self): self.assertEqual(clamp(3, 1, 5), 3)\n    def test_lower(self): self.assertEqual(clamp(-1, 1, 5), 1)\n    def test_upper(self): self.assertEqual(clamp(7, 1, 5), 5)\n";
function config() {
  let text =
    '[runtime]\nmax_parallel_tasks = 2\n[scheduler]\nmax_parallel_tasks = 2\nmax_project_tasks = 2\nmax_replans = 0\nmax_total_tasks = 16\nmax_duration_secs = 1200\n[[integration_checks]]\nprogram = "python"\nargs = ["-B","-m","unittest","discover","-s","tests","-v"]\ntimeout_secs = 30\n';
  for (const role of ["planner", "worker", "reviewer"]) {
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
    text += `\n[agents.${role}]\nkind = "codex"\ncommand = ${JSON.stringify(cli)}\nargs = ${JSON.stringify(args)}\nmax_concurrency = 1\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE = "1"\n[[role_assignments]]\nrole = "${role}"\nagent = "${role}"\npreferred = true\n[policies.roles.${role}.filesystem]\nread = ["**"]\nwrite = ${role === "worker" ? '["src/**","tests/**"]' : "[]"}\ndeny_write = ["tests/baseline_result.json"]\n[policies.roles.${role}.commands]\nmode = "restricted"\nallow = ["python","python3","git","rg","Get-Content","Set-Content","Get-ChildItem","Get-Location","Test-Path","powershell","powershell.exe"]\ndeny = ["curl","Invoke-WebRequest","cmd","bash","sh"]\n[policies.roles.${role}.network]\nmode = "deny"\n`;
  }
  text +=
    '\n[policies.roles.test_engineer.filesystem]\nread = ["**"]\nwrite = ["src/**","tests/**"]\ndeny_write = ["tests/baseline_result.json"]\n[policies.roles.test_engineer.commands]\nmode = "restricted"\nallow = ["python","python3","git","rg","Get-Content","Set-Content","Get-ChildItem","Get-Location","Test-Path","powershell","powershell.exe"]\ndeny = ["curl","Invoke-WebRequest","cmd","bash","sh"]\n[policies.roles.test_engineer.network]\nmode = "deny"\n';
  return text;
}
async function prepare(name) {
  if (result.projects[name]) return;
  const folder = path.join(result.fixture, name);
  await fs.mkdir(path.join(folder, "src"), { recursive: true });
  await fs.mkdir(path.join(folder, "tests"), { recursive: true });
  for (const [file, value] of Object.entries({
    ".gitignore": ".factory/\n__pycache__/\n*.pyc\n",
    "src/__init__.py": "",
    "src/math_tools.py": source,
    "tests/test_math_tools.py": tests,
  }))
    await fs.writeFile(path.join(folder, file), value);
  const baseline = run(
    "python",
    ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"],
    folder,
  );
  ok(baseline);
  await fs.writeFile(
    path.join(folder, "tests/baseline_result.json"),
    JSON.stringify(
      {
        schema: 1,
        observedAt: new Date().toISOString(),
        sourceSha256: hash(source),
        testsSha256: hash(tests),
        ...baseline,
      },
      null,
      2,
    ) + "\n",
  );
  for (const argv of [
    ["init", "-q", "-b", "main"],
    ["config", "user.name", "ASF P09 acceptance"],
    ["config", "user.email", "p09@example.invalid"],
    ["add", "."],
    ["commit", "-qm", "disposable baseline"],
  ])
    ok(run("git", argv, folder));
  ok(run(binary, ["init"], folder));
  await fs.writeFile(path.join(folder, ".factory/config.toml"), config());
  result.projects[name] = {
    root: folder,
    initialHead: ok(run("git", ["rev-parse", "HEAD"], folder)),
    baseline,
  };
}
for (const name of ["Native", "Other"]) await prepare(name);
let child, browser, context, page, csrf, base;
async function start() {
  const port = await new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, "127.0.0.1", () => {
      const p = s.address().port;
      s.close(() => resolve(p));
    });
  });
  base = `http://127.0.0.1:${port}`;
  const logfile = `server-${Date.now()}.log`;
  const handle = await fs.open(path.join(output, logfile), "w");
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
      path.join(result.fixture, "registry.sqlite3"),
      "--allow-project-root",
      result.fixture,
    ],
    {
      cwd: result.projects.Native.root,
      windowsHide: true,
      stdio: ["ignore", handle.fd, handle.fd],
    },
  );
  await handle.close();
  result.server = { pid: child.pid, base, logfile };
  await eventually(async () => {
    try {
      return (await fetch(base + "/api/health")).ok;
    } catch {
      return false;
    }
  }, "owned server");
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
}
async function mutate(url, data, expected = 200, method = "POST") {
  const r = await context.request.fetch(base + url, {
    method,
    data,
    headers: { Origin: base, "X-Factory-CSRF": csrf },
  });
  const text = await r.text();
  assert.equal(r.status(), expected, text);
  return expected === 204 ? null : JSON.parse(text);
}
async function get(item, suffix) {
  const r = await context.request.get(
    `${base}/api/projects/${item.project}/${suffix}`,
  );
  assert.ok(r.ok(), await r.text());
  return r.json();
}
async function register() {
  const r = await context.request.get(base + "/api/projects");
  const entries = await r.json();
  const canonical = (v) =>
    v
      .replace(/^\\\\\?\\/, "")
      .replaceAll("\\", "/")
      .toLowerCase();
  for (const [name, item] of Object.entries(result.projects)) {
    item.project ??= entries.find(
      (e) => canonical(e.canonical_root) === canonical(item.root),
    )?.project_id;
    if (!item.project)
      item.project = (
        await mutate(
          "/api/projects/register",
          { path: item.root, display_name: `P09 ${name}`, trust_project: true },
          201,
        )
      ).project_id;
    await mutate(`/api/projects/${item.project}/open`, {});
  }
  await persist();
}
async function shot(name) {
  await page.screenshot({
    path: path.join(output, "screenshots", name + ".png"),
    fullPage: true,
  });
  if (!result.screenshots.includes(name)) result.screenshots.push(name);
}
async function view(label) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name: label, exact: true }).click();
}
async function select(item) {
  await page.getByLabel("Open registered project").selectOption(item.project);
}
async function settled() {
  await eventually(
    async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    "durable workspace save",
  );
}
async function inspectProfiles() {
  assert.ok(result.setupQualified);
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").waitFor();
  await select(result.projects.Native);
  await view("Agents");
  await settled();
  await page.reload();
  await page
    .getByRole("button", { name: "Edit Implementation A", exact: true })
    .waitFor();
  await settled();
  await shot("agent-list-settled");
  const row = page
    .locator(".asf-agent-row")
    .filter({ has: page.locator("small", { hasText: result.profiles.a.id }) });
  await row.locator("summary").click();
  await row.locator("pre").waitFor();
  await shot("effective-permissions-and-history");
  await select(result.projects.Other);
  await view("Agents");
  await settled();
  await shot("fifty-profiles-settled");
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await shot("agent-management-settled-light");
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await page.setViewportSize({ width: 1050, height: 850 });
  await settled();
  await shot("agent-management-settled-narrow");
  result.layoutInspection = {
    at: new Date().toISOString(),
    qualified:
      "Profile persistence and screenshots after confirmed durable layout save; initial fast-reload warning evidence retained",
  };
  await persist();
}
const programList =
  "python, python3, git, rg, Get-Content, Set-Content, Get-ChildItem, Get-Location, Test-Path, powershell, powershell.exe";
const common =
  "Respect the existing Rust policy, native sandbox and structured output contract. No publication, downloads, staging, commits, policy changes or approval bypass. ASF owns commits/review/integration. Use direct individual allowed programs, without PowerShell assignments, pipelines or command bundles. Report every executed command as its exact unannotated executable invocation. Do not edit tests/baseline_result.json, which contains genuine pre-agent baseline evidence.";
async function createProfile(
  key,
  name,
  role,
  installation,
  effort,
  write,
  preferred,
  instructions,
) {
  if (result.profiles[key]) return result.profiles[key];
  await view("Agents");
  const host = page.locator('[aria-label="Agent management"]:visible').last();
  await host.getByRole("button", { name: "New agent", exact: true }).click();
  const form = host.getByRole("form", { name: "Agent profile editor" });
  await form.getByLabel("Agent name", { exact: true }).fill(name);
  await form
    .getByLabel("Description", { exact: true })
    .fill("Original disposable P09 qualification profile");
  await form
    .getByLabel("Provider installation", { exact: true })
    .selectOption(installation);
  await form
    .getByLabel("Model identifier", { exact: true })
    .fill("gpt-6.1-sol");
  await form
    .getByLabel("Reasoning effort", { exact: true })
    .selectOption(effort);
  if (role !== "worker") {
    await form.getByRole("checkbox", { name: "Worker", exact: true }).uncheck();
    await form
      .getByRole("checkbox", {
        name: role === "planner" ? "Planner" : "Reviewer",
        exact: true,
      })
      .check();
  }
  if (key === "b") {
    await form
      .getByRole("checkbox", { name: "Test Engineer", exact: true })
      .check();
    await form
      .getByLabel("Eligible for automatic delegation", { exact: true })
      .check();
  }
  await form
    .getByLabel("Agent instructions", { exact: true })
    .fill(instructions + " " + common);
  await form
    .getByLabel("Write scopes (comma separated)", { exact: true })
    .fill(write ? "src/**, tests/**" : "");
  await form
    .getByLabel("Denied write scopes", { exact: true })
    .fill("tests/baseline_result.json");
  await form.getByLabel("Approved programs", { exact: true }).fill(programList);
  await form
    .getByLabel("Denied programs", { exact: true })
    .fill("curl, Invoke-WebRequest, cmd, bash, sh");
  await form
    .getByLabel("Execution deadline (seconds)", { exact: true })
    .fill("300");
  if (preferred) {
    await form
      .getByLabel("Eligible for automatic delegation", { exact: true })
      .check();
    await form
      .getByLabel("Preferred for assigned roles", { exact: true })
      .check();
  }
  if (key === "a") {
    await form
      .getByLabel("Model identifier", { exact: true })
      .scrollIntoViewIfNeeded();
    await shot("profile-provider-model");
    await form
      .getByLabel("Agent instructions", { exact: true })
      .scrollIntoViewIfNeeded();
    await shot("profile-instructions");
    await form
      .getByLabel("Write scopes (comma separated)", { exact: true })
      .scrollIntoViewIfNeeded();
    await shot("profile-permission-restrictions");
  }
  const response = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" &&
      new URL(r.url()).pathname.endsWith("/agent-profiles"),
  );
  await form
    .getByRole("button", { name: "Save agent profile", exact: true })
    .click();
  const r = await response;
  assert.ok(r.ok(), await r.text());
  const profile = await r.json();
  await host
    .getByRole("button", { name: `Edit ${name}`, exact: true })
    .waitFor();
  result.profiles[key] = profile;
  await persist();
  return profile;
}
async function setup() {
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").waitFor();
  await select(result.projects.Native);
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await createProfile(
    "a",
    "Implementation A",
    "worker",
    "worker",
    "low",
    true,
    true,
    "Implement carefully scoped source improvements and regression tests.",
  );
  await createProfile(
    "b",
    "Implementation B",
    "worker",
    "worker",
    "medium",
    true,
    false,
    "Inspect source independently. When handoff evidence exists, verify the preserved unapproved work and add independent adversarial regression tests in a new test file named for the current task; correct remaining issues rather than trusting earlier claims.",
  );
  await createProfile(
    "reviewer",
    "Independent review",
    "reviewer",
    "reviewer",
    "low",
    false,
    true,
    "Independently inspect the actual captured source diff, test evidence and current behavior. Run appropriate permitted verification. Approve only valid changes; never modify source or fabricate a test outcome.",
  );
  await createProfile(
    "planner",
    "Mission planner",
    "planner",
    "planner",
    "low",
    false,
    true,
    "Respond conversationally or produce the requested typed plan/action contract; policy is authoritative.",
  );
  await createProfile(
    "readonly",
    "Read-only observer",
    "worker",
    "worker",
    "low",
    false,
    false,
    "Inspect only; no source writes.",
  );
  await settled();
  await page.reload();
  await view("Agents");
  await page
    .getByRole("button", { name: "Edit Implementation A", exact: true })
    .waitFor();
  await shot("agent-list-persisted");
  const id = result.profiles.a.id;
  const details = await get(result.projects.Native, `agent-profiles/${id}`);
  assert.equal(details.profile.modelId, "gpt-6.1-sol");
  assert.equal(details.profile.revision, 1);
  const response = await context.request.get(
    `${base}/api/projects/${result.projects.Other.project}/agent-profiles/${id}`,
  );
  assert.equal(response.status(), 404);
  result.setupQualified = new Date().toISOString();
  await persist();
}
async function benchmark() {
  const item = result.projects.Other;
  const samples = { create: [], list: [], edit: [], policy: [], providers: [] };
  const payload = {
    name: "Implementation A",
    description: "Disabled API/browser scale fixture, not a model execution",
    providerAdapterId: "worker",
    modelId: null,
    reasoningEffort: null,
    roles: ["worker"],
    instructions: "No execution authorized in this benchmark fixture.",
    permissions: { filesystem: { read: ["**"], write: [], deny_write: [] } },
    maxConcurrency: 1,
    timeoutSecs: 30,
    enabled: false,
    automaticDelegation: false,
    preferred: false,
  };
  let target;
  for (let i = 0; i < 50; i++) {
    const start = performance.now();
    target = await mutate(`/api/projects/${item.project}/agent-profiles`, {
      ...payload,
      name: i === 0 ? payload.name : `Bench ${i}`,
    });
    samples.create.push(performance.now() - start);
  }
  for (let i = 0; i < 10; i++) {
    let start = performance.now();
    const profiles = await get(item, "agent-profiles");
    samples.list.push(performance.now() - start);
    assert.equal(profiles.length, 50);
    assert.ok(
      profiles.every((p) => p.projectId === item.project && !p.enabled),
    );
    start = performance.now();
    await get(item, `agent-profiles/${target.id}`);
    samples.policy.push(performance.now() - start);
    start = performance.now();
    await get(item, "agent-profiles/providers");
    samples.providers.push(performance.now() - start);
    const {
      id,
      projectId,
      revision,
      createdAt,
      updatedAt,
      archived,
      ...configuration
    } = target;
    start = performance.now();
    target = await mutate(
      `/api/projects/${item.project}/agent-profiles/${id}`,
      {
        expectedRevision: revision,
        configuration: {
          ...configuration,
          description: `Measured revision ${i}`,
        },
      },
      200,
      "PATCH",
    );
    samples.edit.push(performance.now() - start);
  }
  const {
    id,
    projectId,
    revision,
    createdAt,
    updatedAt,
    archived,
    ...configuration
  } = target;
  const responses = await Promise.all(
    [1, 2].map((n) =>
      context.request.patch(
        `${base}/api/projects/${item.project}/agent-profiles/${id}`,
        {
          headers: { Origin: base, "X-Factory-CSRF": csrf },
          data: {
            expectedRevision: revision,
            configuration: {
              ...configuration,
              description: `Concurrent writer ${n}`,
            },
          },
        },
      ),
    ),
  );
  assert.deepEqual(responses.map((r) => r.status()).sort(), [200, 409]);
  await select(item);
  await view("Agents");
  const host = page
    .locator(
      `[data-project-id="${item.project}"] [aria-label="Agent management"]:visible`,
    )
    .last();
  await eventually(
    async () => (await host.locator(".asf-agent-row").count()) === 50,
    "fifty real backend profiles in UI",
  );
  await shot("fifty-project-owned-profiles");
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await shot("profile-management-light");
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await page.setViewportSize({ width: 1050, height: 850 });
  await shot("profile-management-narrow");
  await page.setViewportSize({ width: 1440, height: 1000 });
  const summarize = (values) => ({
    samples: values.length,
    medianMs: [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)],
    maxMs: Math.max(...values),
  });
  const database = await fs.stat(path.join(item.root, ".factory/db.sqlite3"));
  let wal;
  try {
    wal = (await fs.stat(path.join(item.root, ".factory/db.sqlite3-wal"))).size;
  } catch {
    wal = 0;
  }
  result.profileBenchmark = {
    observedAt: new Date().toISOString(),
    profiles: 50,
    metrics: Object.fromEntries(
      Object.entries(samples).map(([k, v]) => [k, summarize(v)]),
    ),
    samples,
    conflictingEdits: responses.map((r) => r.status()),
    databaseBytes: database.size,
    walBytes: wal,
    rendererHeap: await page.evaluate(() =>
      performance.memory
        ? {
            used: performance.memory.usedJSHeapSize,
            total: performance.memory.totalJSHeapSize,
            limit: performance.memory.jsHeapSizeLimit,
          }
        : null,
    ),
    modelInvocations: 0,
    qualification:
      "Real authenticated API and production browser; disabled profiles; no agent invocation",
  };
  await select(result.projects.Native);
  await view("Agents");
  await persist();
}
const panel = () => page.locator(".chat-panel:visible").last();
async function nativeAcceptance() {
  assert.ok(
    result.setupQualified,
    "Run the model-free setup/browser qualification first",
  );
  assert.ok(
    !result.nativeStarted,
    "A previous native attempt is retained; do not blindly repeat it",
  );
  const preflight = JSON.parse(
    await fs.readFile(
      path.join(root, ".factory/v2/p09/preflight/preflight.json"),
      "utf8",
    ),
  );
  assert.equal(preflight.write, "PASS");
  assert.equal(preflight.exit, 0);
  result.nativeStarted = new Date().toISOString();
  await persist();
  const item = result.projects.Native;
  await page.goto(base + "/workspace.html");
  await page.getByLabel("Open registered project").waitFor();
  await select(item);
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await view("Orchestrator");
  const creation = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" &&
      new URL(r.url()).pathname.endsWith("/conversations"),
  );
  await panel()
    .getByRole("button", { name: "New conversation", exact: true })
    .click();
  item.conversation = (await (await creation).json()).id;
  await panel()
    .getByLabel("Message to orchestrator", { exact: true })
    .fill(
      "Explain in one sentence why task worktrees and independent review protect a development mission. This is a question only; do not start a mission or modify files.",
    );
  await panel().getByRole("button", { name: "Send", exact: true }).click();
  await eventually(
    async () => {
      const messages = await get(
        item,
        `conversations/${item.conversation}/messages?limit=50`,
      );
      const answer = messages.find(
        (m) => m.role === "orchestrator" && m.status === "finished",
      );
      if (
        messages.some(
          (m) =>
            m.status === "failed" &&
            /usage limit|resource.exhausted/i.test(m.content),
        )
      ) {
        result.modelAcceptance = "BLOCKED — PROVIDER USAGE LIMIT";
        throw new Error(result.modelAcceptance);
      }
      if (answer) {
        result.conversationResponse = answer;
        return true;
      }
      return false;
    },
    "real configured Planner conversation response",
    240000,
  );
  const objective =
    "Extend this small Python project with strict finite-bound clamp validation and an independent structured event formatter. Preserve valid existing behavior, add meaningful tests, run the suite, and leave review and integration to ASF. Only src/** and tests/** may change. " +
    common;
  const tasks = [
    {
      title: "Finite clamp validation",
      objective:
        "Extend src/math_tools.py clamp(value, lower, upper) to raise ValueError for NaN, positive or negative infinity in any argument, and for reversed bounds. Preserve all valid finite behavior, including equal bounds and negative inputs. Add regression tests and run the actual suite.",
      role: "worker",
      operation: "implement",
      dependencies: [],
    },
    {
      title: "Structured event formatter",
      objective:
        "Add src/events.py exposing format_event(level, message, fields=None). Return deterministic sorted-key JSON with exactly level, message and fields. Accept DEBUG, INFO, WARNING and ERROR case-insensitively and output uppercase. Require a string message and a dict or None fields value; do not mutate the caller dictionary. An invalid level raises ValueError; invalid message or fields types raise TypeError. Add meaningful tests and run them. This module is independent of clamp validation.",
      role: "test_engineer",
      operation: "implement",
      dependencies: [],
    },
  ];
  await panel()
    .getByRole("button", { name: "Visual plan", exact: true })
    .click();
  await panel()
    .getByLabel("Mission objective", { exact: true })
    .fill(objective);
  await panel()
    .getByLabel("Scope", { exact: true })
    .fill("src/** and tests/**; preserve tests/baseline_result.json");
  await panel()
    .getByLabel("Constraints", { exact: true })
    .fill(
      "No publication, policy change, staging, commit, download or approval bypass. Independent review is mandatory.",
    );
  await panel()
    .locator("summary")
    .filter({ hasText: "Preferred configured agents" })
    .click();
  await panel()
    .getByLabel("Preferred worker", { exact: true })
    .selectOption(result.profiles.a.id);
  await panel()
    .getByLabel("Preferred reviewer", { exact: true })
    .selectOption(result.profiles.reviewer.id);
  for (let i = 0; i < tasks.length; i++) {
    const t = tasks[i],
      id = `T${i + 1}`;
    await panel()
      .getByRole("button", { name: "Add task", exact: true })
      .click();
    await panel().getByLabel(`Title ${id}`, { exact: true }).fill(t.title);
    await panel()
      .getByLabel(`Objective ${id}`, { exact: true })
      .fill(t.objective);
    await panel()
      .getByLabel(`Role ${id}`, { exact: true })
      .selectOption(t.role);
    await panel()
      .getByLabel(`Operation ${id}`, { exact: true })
      .selectOption(t.operation);
    await panel()
      .getByLabel(`Acceptance ${id}`, { exact: true })
      .fill(
        "Relevant nonempty source changes; real regression tests; existing tests pass; actual review before integration",
      );
  }
  await shot("custom-agent-authored-mission");
  await panel()
    .getByRole("button", { name: "Validate and launch plan", exact: true })
    .click();
  const mission = await eventually(async () => {
    const m = await get(item, `conversations/${item.conversation}/missions`);
    return m.find((m) => m.tasks?.length === 2);
  }, "mission persistence");
  item.run = mission.run_id;
  item.objective = objective;
  item.tasks = tasks;
  await persist();
  const detail = () => get(item, `runs/${item.run}`);
  const initial = await eventually(
    async () => {
      const d = await detail();
      const s = d.sessions.find(
        (s) =>
          s.role === "worker" &&
          s.agent === result.profiles.a.id &&
          s.status === "running" &&
          s.outputMeta?.processId,
      );
      return s && { detail: d, session: s };
    },
    "genuine profile A Worker",
    120000,
  );
  assert.equal(initial.session.outputMeta.processTreeSupervised, true);
  const firstTask = initial.session.taskId;
  item.initialWorker = {
    id: initial.session.id,
    taskId: firstTask,
    process: initial.session.outputMeta,
    observedAt: new Date().toISOString(),
  };
  const readonlyBody = {
    idempotencyKey: "p09-denied-readonly",
    toProfileId: result.profiles.readonly.id,
    expectedProfileRevision: 1,
    reason:
      "Negative acceptance: this read-only profile cannot perform implementation.",
  };
  item.rejectedReplacement = await mutate(
    `/api/projects/${item.project}/runs/${item.run}/tasks/${firstTask}/handoff`,
    readonlyBody,
    403,
  );
  assert.equal(item.rejectedReplacement.code, "permission_denied");
  await view("Progress");
  const handoff = page.locator(".asf-handoffs:visible").last();
  await handoff.locator("summary").first().click();
  await handoff
    .getByLabel("Task to reassign", { exact: true })
    .selectOption(String(firstTask));
  await handoff
    .getByLabel("Replacement agent", { exact: true })
    .selectOption(result.profiles.readonly.id);
  await handoff
    .getByLabel("Handoff reason", { exact: true })
    .fill("Verify that a read-only profile cannot replace a writable Worker.");
  await handoff
    .getByRole("button", { name: "Request safe handoff", exact: true })
    .click();
  await handoff.getByRole("alert").waitFor();
  await shot("rejected-readonly-replacement");
  await handoff
    .getByLabel("Replacement agent", { exact: true })
    .selectOption(result.profiles.b.id);
  await handoff
    .getByLabel("Handoff reason", { exact: true })
    .fill(
      "Safely transfer the active clamp task after current Worker exit. Independently verify preserved work and add adversarial regression coverage.",
    );
  const request = page.waitForResponse(
    (r) =>
      r.request().method() === "POST" &&
      new URL(r.url()).pathname.endsWith(`/tasks/${firstTask}/handoff`),
  );
  await handoff
    .getByRole("button", { name: "Request safe handoff", exact: true })
    .click();
  const response = await request;
  assert.equal(response.status(), 200, await response.text());
  item.runningHandoff = await response.json();
  assert.equal(item.runningHandoff.state, "waiting");
  const body = response.request().postDataJSON();
  const repeated = await mutate(
    `/api/projects/${item.project}/runs/${item.run}/tasks/${firstTask}/handoff`,
    body,
  );
  assert.equal(repeated.id, item.runningHandoff.id);
  await shot("pending-real-worker-handoff");
  await persist();
  let last = "";
  await eventually(
    async () => {
      const d = await detail();
      const signature = JSON.stringify([
        d.run.status,
        d.sessions.map((s) => [s.id, s.status]),
        d.tasks.map((t) => [t.id, t.state]),
      ]);
      if (signature !== last) {
        last = signature;
        result.timeline ??= [];
        result.timeline.push({
          at: new Date().toISOString(),
          runStatus: d.run.status,
          tasks: d.tasks.map((t) => ({ id: t.id, state: t.state })),
          sessions: d.sessions.map((s) => ({
            id: s.id,
            agent: s.agent,
            role: s.role,
            status: s.status,
            taskId: s.taskId,
            process: s.outputMeta,
          })),
        });
        await persist();
        console.log(
          `${d.run.status}: ${d.sessions.map((s) => `${s.role}/${s.agent.slice(-6)}:${s.status}`).join(", ")}`,
        );
      }
      const live = d.sessions.filter(
        (s) =>
          ["worker", "test_engineer"].includes(s.role) &&
          s.status === "running",
      );
      if (
        live.some((s) => s.agent === result.profiles.a.id) &&
        live.some((s) => s.agent === result.profiles.b.id) &&
        !item.parallelObserved
      ) {
        item.parallelObserved = {
          at: new Date().toISOString(),
          sessions: live.map((s) => ({
            id: s.id,
            agent: s.agent,
            taskId: s.taskId,
            process: s.outputMeta,
          })),
        };
        await shot("two-real-custom-workers");
      }
      if (
        d.sessions.some(
          (s) =>
            s.status === "blocked" &&
            /usage limit|resource.exhausted/i.test(s.stderr ?? ""),
        )
      ) {
        result.modelAcceptance = "BLOCKED — PROVIDER USAGE LIMIT";
        throw new Error(result.modelAcceptance);
      }
      if (
        ["completed", "failed", "cancelled", "blocked"].includes(d.run.status)
      ) {
        item.finalDetail = d;
        return true;
      }
      return false;
    },
    "real custom-agent mission completion",
    1000000,
  );
  const final = item.finalDetail;
  assert.equal(
    final.run.status,
    "completed",
    JSON.stringify(
      final.attempts.map((a) => ({
        status: a.status,
        error: a.error,
        review: a.review,
      })),
    ),
  );
  item.handoffs = await get(item, `runs/${item.run}/agent-handoffs`);
  assert.equal(item.handoffs.filter((h) => h.state === "completed").length, 1);
  assert.ok(
    item.handoffs.some(
      (h) =>
        h.state === "rejected" && h.target.id === result.profiles.readonly.id,
    ),
  );
  const workers = final.sessions.filter((s) =>
    ["worker", "test_engineer"].includes(s.role),
  );
  assert.equal(workers.length, 3);
  assert.equal(new Set(workers.map((s) => s.taskId)).size, 2);
  const old = workers.find((s) => s.id === initial.session.id);
  const replacement = workers.find(
    (s) =>
      s.id ===
      item.handoffs.find((h) => h.id === item.runningHandoff.id)
        .replacementSessionId,
  );
  assert.ok(replacement && replacement.agent === result.profiles.b.id);
  assert.ok(
    old.outputMeta.processFinishedUnixMs <=
      replacement.outputMeta.processStartedUnixMs,
  );
  const parallel = workers.find((s) => s.taskId !== firstTask);
  item.overlapMs =
    Math.min(
      old.outputMeta.processFinishedUnixMs,
      parallel.outputMeta.processFinishedUnixMs,
    ) -
    Math.max(
      old.outputMeta.processStartedUnixMs,
      parallel.outputMeta.processStartedUnixMs,
    );
  assert.ok(item.overlapMs > 0, "Actual process intervals must overlap");
  assert.notEqual(
    final.attempts.find((a) => a.id === old.attemptId).worktreePath,
    final.attempts.find((a) => a.id === parallel.attemptId).worktreePath,
  );
  item.executionEvidence = [];
  for (const session of final.sessions) {
    const snapshot = await get(
      item,
      `agent-sessions/${session.id}/execution-snapshot`,
    );
    const observation = await get(
      item,
      `agent-sessions/${session.id}/model-observation`,
    );
    assert.ok(snapshot && snapshot.profile.projectId === item.project);
    assert.equal(snapshot.requestedModel, "gpt-6.1-sol");
    assert.equal(observation.model, "gpt-6.1-sol");
    if (session.role === "reviewer") {
      assert.equal(snapshot.profile.id, result.profiles.reviewer.id);
      assert.equal(snapshot.effectivePolicy.filesystemMode, "read_only");
    }
    item.executionEvidence.push({
      ...session,
      stderr: undefined,
      stdout: session.stdout,
      stderrSha256: hash(session.stderr ?? ""),
      nativeHeader: (session.stderr ?? "")
        .split(/\r?\n/)
        .slice(0, 40)
        .filter((s) =>
          /^(OpenAI Codex|workdir:|model:|provider:|approval:|sandbox:|reasoning effort:)/.test(
            s,
          ),
        ),
      snapshot,
      observation,
    });
  }
  assert.equal(
    final.attempts.filter((a) => a.status === "handed_off").length,
    1,
  );
  assert.equal(
    final.attempts.filter(
      (a) => a.status === "approved" && a.review?.decision === "approve",
    ).length,
    2,
  );
  item.integration = await get(item, `runs/${item.run}/scheduler`);
  assert.equal(item.integration.activeClaims, 0);
  assert.ok(
    item.integration.integrations.filter((r) => r.status === "published")
      .length === 2,
  );
  item.integrationCommit = final.integration.head;
  assert.ok(item.integrationCommit);
  const checkout = path.join(result.fixture, "independent-checkout");
  ok(
    run(
      "git",
      ["worktree", "add", "--detach", checkout, item.integrationCommit],
      item.root,
    ),
  );
  item.verification = {
    checkout,
    tests: run(
      "python",
      ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"],
      checkout,
    ),
  };
  ok(item.verification.tests);
  const oracle = `import math,json\nfrom src.math_tools import clamp\nfrom src.events import format_event\ncount=0\nfor args in [(3,1,5),(-9,-5,-1),(0,0,0),(9,1,5)]:\n assert clamp(*args)==min(max(args[0],args[1]),args[2]);count+=1\nfor position in range(3):\n for bad in (float('nan'),float('inf'),float('-inf')):\n  args=[2,1,3];args[position]=bad\n  try: clamp(*args)\n  except ValueError: count+=1\n  else: raise AssertionError(args)\ntry: clamp(2,3,1)\nexcept ValueError: count+=1\nelse: raise AssertionError('reversed bounds accepted')\nfields={'b':2,'a':1};old=fields.copy();encoded=format_event('info','hello',fields);assert json.loads(encoded)=={'level':'INFO','message':'hello','fields':fields};assert old==fields;assert encoded==format_event('INFO','hello',{'a':1,'b':2});count+=3\nfor args,expected in [(('bogus','message',None),ValueError),(('INFO',3,None),TypeError),(('INFO','message',[]),TypeError)]:\n try: format_event(*args)\n except expected: count+=1\n else: raise AssertionError(args)\nprint('Independent behavioral assertions:',count)\n`;
  item.verification.oracle = run("python", ["-B", "-c", oracle], checkout);
  ok(item.verification.oracle);
  item.diff = ok(
    run(
      "git",
      ["diff", item.initialHead, item.integrationCommit, "--", "src", "tests"],
      item.root,
    ),
  );
  assert.ok(
    item.diff.includes("src/math_tools.py") &&
      item.diff.includes("src/events.py"),
  );
  assert.equal(
    ok(run("git", ["rev-parse", "HEAD"], item.root)),
    item.initialHead,
  );
  assert.equal(ok(run("git", ["status", "--porcelain"], item.root)), "");
  assert.equal(ok(run("git", ["remote", "-v"], item.root)), "");
  const roots = await get(item, "files/roots");
  const integration = roots.roots.find(
    (r) => r.kind === "integration_snapshot",
  );
  assert.ok(integration?.read_only);
  const file = await get(
    item,
    `files/content?root_id=${integration.id}&path=src/math_tools.py`,
  );
  assert.equal(
    file.content,
    await fs.readFile(path.join(checkout, "src/math_tools.py"), "utf8"),
  );
  await view("Explorer");
  await page
    .getByLabel("Filesystem root", { exact: true })
    .selectOption(integration.id);
  const tree = page.locator(".files-explorer:visible").last().getByRole("tree");
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
  await page
    .locator(".cm-content")
    .filter({ hasText: "clamp" })
    .first()
    .waitFor();
  await shot("integrated-profile-code-readonly");
  await view("Progress");
  await shot("completed-real-handoff");
  await settled();
  await page.reload();
  await page.getByLabel("Open registered project").waitFor();
  await select(item);
  await view("Progress");
  await shot("restored-handoff-history");
  await view("Agents");
  const host = page.locator('[aria-label="Agent management"]:visible').last();
  await host
    .locator(".asf-agent-row")
    .filter({ has: page.locator("small", { hasText: result.profiles.a.id }) })
    .getByRole("button", { name: "Disable", exact: true })
    .click();
  await eventually(async () => {
    const p = await get(item, `agent-profiles/${result.profiles.a.id}`);
    return !p.profile.enabled;
  }, "disabled profile persisted");
  await shot("agent-disabled-history-retained");
  assert.ok(
    (await get(item, `agents/${result.profiles.a.id}/sessions`)).length > 0,
  );
  await page.getByLabel("Theme", { exact: true }).selectOption("light");
  await shot("agent-management-light");
  await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  await shot("agent-management-dark");
  const messages = await get(
    item,
    `conversations/${item.conversation}/messages?limit=50`,
  );
  item.resultMessages = messages.filter(
    (m) => m.provenance?.source === "runtime",
  );
  assert.ok(
    item.resultMessages.some(
      (m) => m.provenance?.snapshot?.status === "completed",
    ),
  );
  assert.equal(result.pageErrors.length, 0, JSON.stringify(result.pageErrors));
  delete item.finalDetail;
  item.qualified = new Date().toISOString();
  result.modelAcceptance =
    "PASS — CODEX LOW/MEDIUM CONFIGURATIONS; CROSS-PROVIDER EXECUTION NOT QUALIFIED";
  await persist();
}
async function cleanup() {
  if (browser) await browser.close();
  if (child && child.exitCode === null && child.signalCode === null) {
    child.kill();
    await new Promise((resolve) => child.once("exit", resolve));
  }
  await persist();
}
try {
  await start();
  await register();
  if (inspect) await inspectProfiles();
  else if (!native) {
    await setup();
    await benchmark();
  } else await nativeAcceptance();
} catch (error) {
  result.failure = String(error);
  await persist();
  throw error;
} finally {
  await cleanup();
}
