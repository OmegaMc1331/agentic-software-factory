import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import { createHash } from "node:crypto";
import os from "node:os";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const axePath = require.resolve("axe-core/axe.min.js");
const output = path.resolve(
    root,
    process.env.ASF_P05_OUTPUT_DIR ?? ".factory/v2/p05",
  ),
  shots = path.join(output, "screenshots");
await fs.mkdir(shots, { recursive: true });
const binary =
  process.env.ASF_P05_BINARY ??
  path.join(
    root,
    "target/debug",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const fixture = await fs.mkdtemp(path.join(output, "editor-fixtures-")),
  a = path.join(fixture, "Project A"),
  b = path.join(fixture, "Project B"),
  registry = path.join(
    await fs.mkdtemp(path.join(os.tmpdir(), "asf-p05-application-")),
    "registry.sqlite3",
  );
const results = {
  schema: 1,
  started: new Date().toISOString(),
  binary,
  fixture,
  tests: [],
  screenshots: [],
  measurements: [],
  errors: [],
  requests: [],
  serverPids: [],
  platform: process.platform,
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
results.sourceCommit = run("git", ["rev-parse", "HEAD"]).trim();
results.scriptSha256 = createHash("sha256")
  .update(await fs.readFile(fileURLToPath(import.meta.url)))
  .digest("hex");
results.binarySha256 = createHash("sha256")
  .update(await fs.readFile(binary))
  .digest("hex");
for (const [directory, label] of [
  [a, "A"],
  [b, "B"],
]) {
  await fs.mkdir(path.join(directory, "src/nested"), { recursive: true });
  run("git", ["init", "-q", "-b", "main"], directory);
  run(binary, ["init"], directory);
  await fs.writeFile(
    path.join(directory, ".gitignore"),
    ".factory/\nignored/\n",
  );
  await fs.writeFile(
    path.join(directory, "src/main.rs"),
    `fn main() {\n    println!("Project ${label}");\n}\n`,
  );
  await fs.writeFile(
    path.join(directory, "src/nested/module.ts"),
    "export const value = 'needle';\n",
  );
  await fs.writeFile(
    path.join(directory, "unicode-é.py"),
    Buffer.from("\ufeffvalue = 'été'\r\nprint(value)"),
  );
  await fs.writeFile(
    path.join(directory, "binary.bin"),
    Buffer.from([0, 255, 34]),
  );
  await fs.writeFile(path.join(directory, "large.txt"), "x".repeat(1_048_577));
  run(
    "git",
    [
      "-c",
      "user.name=P05 fixture",
      "-c",
      "user.email=p05@example.test",
      "add",
      ".",
    ],
    directory,
  );
  run(
    "git",
    [
      "-c",
      "user.name=P05 fixture",
      "-c",
      "user.email=p05@example.test",
      "commit",
      "-qm",
      "P05 fixture",
    ],
    directory,
  );
}
const worktree = path.join(a, ".factory/worktrees/t42");
run("git", ["worktree", "add", "--detach", worktree, "HEAD"], a);
const port = await new Promise((resolve) => {
  const server = net.createServer();
  server.listen(0, "127.0.0.1", () => {
    const port = server.address().port;
    server.close(() => resolve(port));
  });
});
const base = `http://127.0.0.1:${port}`;
const serverEnvironment = { ...process.env };
if (process.platform === "win32") {
  const git = run("where.exe", ["git"]).trim().split(/\r?\n/)[0],
    system = process.env.SystemRoot ?? "C:\\Windows";
  for (const key of Object.keys(serverEnvironment))
    if (key.toLowerCase() === "path") delete serverEnvironment[key];
  serverEnvironment.PATH = [
    path.dirname(git),
    path.join(system, "System32"),
    system,
  ].join(path.delimiter);
  results.serverPath = serverEnvironment.PATH;
}
let child, browser, context, page;
async function eventually(action, message = "condition", timeout = 20000) {
  const until = Date.now() + timeout;
  let error;
  while (Date.now() < until) {
    try {
      const value = await action();
      if (value) return value;
    } catch (reason) {
      error = reason;
    }
    await new Promise((resolve) => setTimeout(resolve, 70));
  }
  throw new Error(`${message}: ${error?.message ?? "deadline"}`);
}
async function start() {
  const log = await fs.open(path.join(output, "editor-server.log"), "a");
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
  await eventually(async () => {
    if (child.exitCode !== null)
      throw new Error(`server exited ${child.exitCode}`);
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  }, "server start");
}
async function stop() {
  if (child && child.exitCode === null)
    await new Promise((resolve) => {
      child.once("exit", resolve);
      child.kill();
    });
}
async function auth() {
  const response = await context.request.post(`${base}/api/auth/session`, {
    headers: { Origin: base },
    data: {},
  });
  assert.ok(response.ok(), await response.text());
  return (await response.json()).csrfToken;
}
async function mutate(url, method, data, allowError = false) {
  const csrf = await auth();
  const response = await context.request.fetch(`${base}/api${url}`, {
    method,
    headers: { Origin: base, "X-Factory-CSRF": csrf },
    data,
  });
  if (!allowError) assert.ok(response.ok(), await response.text());
  return { status: response.status(), value: await response.json() };
}
async function read(url) {
  const response = await context.request.get(`${base}/api${url}`);
  assert.ok(response.ok(), await response.text());
  return response.json();
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
    await page
      ?.screenshot({ path: path.join(shots, "failure.png") })
      .catch(() => {});
    if (page) {
      results.persistenceFailure = await page
        .evaluate(() => ({
          workspaceId: localStorage.getItem("asf.v2.workspace.last"),
          recovery: sessionStorage.getItem("asf.v2.workspace.recovery"),
          status: document.querySelector(".ws-status")?.textContent,
        }))
        .catch(() => null);
      if (results.persistenceFailure?.workspaceId) {
        results.persistenceFailure.server = await read(
          `/workspaces/${results.persistenceFailure.workspaceId}`,
        ).catch(() => null);
      }
    }
    throw error;
  } finally {
    await fs.writeFile(
      path.join(output, "editor-browser-results.json"),
      JSON.stringify(results, null, 2),
    );
  }
}
async function shot(name) {
  await page.screenshot({ path: path.join(shots, name) });
  results.screenshots.push(name);
}
async function view(name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
  if (name === "Explorer")
    await eventually(
      async () =>
        (await explorer().getByRole("tree").getAttribute("aria-busy")) ===
        "false",
      "explorer refresh",
    );
}
async function idle() {
  await eventually(
    async () =>
      (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      ),
    "layout saved",
  );
}
async function preserveRecoveredLayout() {
  const conflict = page.getByText(
    "A recovered local layout differs from the server revision. Save an independent copy or reload the server layout.",
    { exact: true },
  );
  if (!(await conflict.isVisible())) return;
  const originalId = await page.evaluate(() =>
    localStorage.getItem("asf.v2.workspace.last"),
  );
  const original = await read(`/workspaces/${originalId}`);
  await page
    .getByRole("button", { name: "Save independent copy", exact: true })
    .click();
  await idle();
  const copyId = await page.evaluate(() =>
    localStorage.getItem("asf.v2.workspace.last"),
  );
  assert.notEqual(copyId, originalId);
  assert.deepEqual(await read(`/workspaces/${originalId}`), original);
  assert.ok((await read(`/workspaces/${copyId}`)).revision > 0);
  (results.layoutRecoveries ??= []).push({
    originalId,
    copyId,
    originalRevision: original.revision,
    originalPreserved: true,
  });
}
async function selectProject(id) {
  await page.getByLabel("Open registered project").selectOption(id);
  await page.locator(`#project-${id}[aria-selected=true]`).waitFor();
}
const explorer = () => page.locator(".files-explorer:visible");
const editor = (filename = "main.rs") =>
  page
    .locator(`.files-editor:visible`)
    .filter({
      has: page.getByRole("toolbar", {
        name: new RegExp(`File actions .*${filename.replaceAll(".", "\\.")}$`),
      }),
    })
    .first();
async function openPath(relative, beside = false) {
  await view("Explorer");
  const tree = explorer().getByRole("tree");
  const parts = relative.split("/");
  let prefix = "";
  for (const [i, part] of parts.entries()) {
    prefix = prefix ? `${prefix}/${part}` : part;
    const row = tree
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", {
          hasText: new RegExp(`^${part.replaceAll(".", "\\.")}$`),
        }),
      })
      .first();
    await tree.evaluate((element) => {
      element.scrollTop = 0;
    });
    await eventually(async () => {
      if (await row.isVisible()) return true;
      await tree.evaluate((element) => {
        const before = element.scrollTop;
        element.scrollTop += Math.max(24, element.clientHeight * 0.75);
        if (element.scrollTop === before) element.scrollTop = 0;
      });
      return false;
    }, `reveal virtualized tree item ${prefix}`);
    await row.waitFor();
    if (i === parts.length - 1) {
      if (beside) {
        await row.click({ button: "right" });
        await page
          .getByRole("menuitem", { name: "Open file beside", exact: true })
          .click();
      } else await row.click();
    } else if ((await row.getAttribute("aria-expanded")) !== "true")
      await row.click();
  }
  await page
    .getByRole("textbox", { name: `Source editor ${relative}`, exact: true })
    .last()
    .waitFor();
}
async function edit(relative, text) {
  const source = page
    .getByRole("textbox", { name: `Source editor ${relative}`, exact: true })
    .last();
  await source.click();
  await source.press("ControlOrMeta+a");
  await source.press("Backspace");
  await source.pressSequentially(text);
}
async function save(filename = "main.rs") {
  await editor(filename)
    .getByRole("button", { name: "Save", exact: true })
    .click();
  await eventually(
    async () =>
      (await editor(filename).locator(".files-status").innerText()).includes(
        "Matches loaded disk revision",
      ),
    "save acknowledgement",
  );
}
let aid, bid, ar, wr;
try {
  await start();
  browser = await chromium.launch({
    channel: process.env.ASF_BROWSER_CHANNEL ?? "msedge",
    headless: true,
  });
  results.browser = browser.version();
  context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: "reduce",
    colorScheme: "dark",
  });
  await auth();
  aid = (await read("/projects"))[0].project_id;
  bid = (
    await mutate("/projects/register", "POST", {
      path: b,
      display_name: "Project B",
      trust_project: true,
    })
  ).value.project_id;
  await mutate(`/projects/${bid}/open`, "POST", {});
  const roots = await read(`/projects/${aid}/files/roots`);
  ar = roots.roots.find((root) => root.kind === "repository").id;
  wr = roots.roots.find((root) => root.kind === "task_worktree").id;
  results.aid = aid;
  results.bid = bid;
  results.roots = roots;
  page = await context.newPage();
  page.setDefaultTimeout(12000);
  page.on("pageerror", (error) => results.errors.push(error.message));
  results.workspaceResponses = [];
  page.on("response", async (response) => {
    const pathname = new URL(response.url()).pathname;
    if (
      !pathname.startsWith("/api/workspaces") ||
      results.workspaceResponses.length >= 100
    )
      return;
    try {
      const value = await response.json();
      const body = response.request().postDataJSON();
      results.workspaceResponses.push({
        pathname,
        method: response.request().method(),
        status: response.status(),
        baseRevision: body?.base_revision,
        revision: value?.revision,
        at: new Date().toISOString(),
      });
    } catch {}
  });
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== base)
      results.requests.push(request.url());
  });
  await page.goto(`${base}/workspace.html`);
  await selectProject(aid);
  await test("EDIT-01 real lazy tree and Rust source", async () => {
    await openPath("src/main.rs");
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .last()
        .innerText(),
      /Project A/,
    );
    await shot("01-real-rust-dark.png");
  });
  await test("EDIT-02 exact disk save", async () => {
    await edit("src/main.rs", 'fn main() { println!("A saved"); }');
    await save();
    assert.equal(
      await fs.readFile(path.join(a, "src/main.rs"), "utf8"),
      'fn main() { println!("A saved"); }',
    );
  });
  await test("EDIT-03 Unicode BOM CRLF and no final newline", async () => {
    await openPath("unicode-é.py");
    await edit("unicode-é.py", "value = 'résolu'");
    await save("unicode-é.py");
    assert.deepEqual(
      await fs.readFile(path.join(a, "unicode-é.py")),
      Buffer.from("\ufeffvalue = 'résolu'"),
    );
  });
  await test("EDIT-04/05 multiple documents and shared split", async () => {
    await openPath("src/nested/module.ts");
    await openPath("src/main.rs", true);
    const panes = page.getByRole("textbox", {
      name: "Source editor src/main.rs",
      exact: true,
    });
    await openPath("src/nested/module.ts");
    assert.ok(
      await page
        .getByRole("textbox", {
          name: "Source editor src/main.rs",
          exact: true,
        })
        .isVisible(),
    );
    assert.ok(
      await page
        .getByRole("textbox", {
          name: "Source editor src/nested/module.ts",
          exact: true,
        })
        .isVisible(),
    );
    await openPath("src/main.rs");
    assert.equal(
      await page
        .getByRole("textbox", {
          name: "Source editor src/main.rs",
          exact: true,
        })
        .count(),
      2,
    );
    await edit("src/main.rs", "shared human draft");
    for (const pane of await page
      .getByRole("textbox", { name: "Source editor src/main.rs", exact: true })
      .all())
      assert.match(await pane.innerText(), /shared human draft/);
    await idle();
    await shot("02-split-editors.png");
  });
  await test("EDIT-06/23 two project buffers remain isolated", async () => {
    await selectProject(bid);
    await openPath("src/main.rs");
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .last()
        .innerText(),
      /Project B/,
    );
    await edit("src/main.rs", "B independent");
    await save();
    assert.equal(
      await fs.readFile(path.join(b, "src/main.rs"), "utf8"),
      "B independent",
    );
    assert.match(
      await fs.readFile(path.join(a, "src/main.rs"), "utf8"),
      /A saved/,
    );
    await selectProject(aid);
    await openPath("src/main.rs");
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .last()
        .innerText(),
      /shared human draft/,
    );
  });
  await test("EDIT-07/29 reload restores dirty draft and layout without disk writes", async () => {
    await eventually(
      async () => (await editor().innerText()).includes("Draft backed up"),
      "draft durable",
    );
    await idle();
    await page.reload();
    await page
      .getByRole("textbox", { name: "Source editor src/main.rs" })
      .last()
      .waitFor();
    assert.match(await editor().innerText(), /Recovered unsaved draft/);
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .last()
        .innerText(),
      /shared human draft/,
    );
    assert.match(
      await fs.readFile(path.join(a, "src/main.rs"), "utf8"),
      /A saved/,
    );
    await shot("03-recovered-draft.png");
  });
  await test("EDIT-09/10/12 external writer triggers conflict and three-way preservation", async () => {
    await fs.writeFile(path.join(a, "src/main.rs"), "agent external version\n");
    await eventually(
      async () => (await editor().innerText()).includes("Disk changed while"),
      "external change detection",
    );
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .last()
        .innerText(),
      /shared human draft/,
    );
    await editor()
      .getByRole("button", { name: "Review differences", exact: true })
      .click();
    await page.locator(".files-merge .cm-mergeView").waitFor();
    assert.match(
      await page.locator(".files-merge .cm-merge-a").innerText(),
      /agent external version/,
    );
    assert.match(
      await page.locator(".files-merge .cm-merge-b").innerText(),
      /shared human draft/,
    );
    await shot("04-external-conflict-diff.png");
  });
  await test("EDIT-10 resolve against reviewed disk revision", async () => {
    await openPath("src/main.rs");
    await editor()
      .getByRole("button", { name: "Save resolved version", exact: true })
      .click();
    await page
      .getByRole("dialog", { name: "Save this manually resolved version?" })
      .getByRole("button", { name: "Save resolved version", exact: true })
      .click();
    await eventually(
      async () =>
        (await fs.readFile(path.join(a, "src/main.rs"), "utf8")) ===
        "shared human draft",
      "resolved saved",
    );
  });
  await test("EDIT-13 create file through real explorer", async () => {
    await view("Explorer");
    await explorer()
      .getByRole("button", { name: "New file", exact: true })
      .click();
    await page
      .getByLabel("Relative destination path", { exact: true })
      .fill("src/created.txt");
    await page
      .getByRole("button", { name: "Apply file operation", exact: true })
      .click();
    await page
      .getByRole("textbox", { name: "Source editor src/created.txt" })
      .waitFor();
    assert.equal(
      await fs.readFile(path.join(a, "src/created.txt"), "utf8"),
      "",
    );
    await edit("src/created.txt", "created and dirty");
  });
  await test("EDIT-14 rename an open dirty file", async () => {
    await view("Explorer");
    await explorer()
      .getByRole("button", { name: "Refresh explorer", exact: true })
      .click();
    await explorer()
      .getByRole("treeitem")
      .filter({ hasText: "created.txt" })
      .click({ button: "right" });
    await page.getByRole("menuitem", { name: "Rename", exact: true }).click();
    await page.getByLabel("New name", { exact: true }).fill("renamed.txt");
    await page
      .getByRole("button", { name: "Apply file operation", exact: true })
      .click();
    await openPath("src/renamed.txt");
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/renamed.txt" })
        .innerText(),
      /created and dirty/,
    );
    await save("renamed.txt");
    assert.equal(
      await fs.readFile(path.join(a, "src/renamed.txt"), "utf8"),
      "created and dirty",
    );
  });
  await test("EDIT-15 move within root preserves file identity and buffer", async () => {
    await view("Explorer");
    await explorer()
      .getByRole("treeitem")
      .filter({ hasText: "renamed.txt" })
      .click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Move within root", exact: true })
      .click();
    await page
      .getByLabel("Relative destination path", { exact: true })
      .fill("src/nested/moved.txt");
    await page
      .getByRole("button", { name: "Apply file operation", exact: true })
      .click();
    await openPath("src/nested/moved.txt");
    assert.equal(
      await fs.readFile(path.join(a, "src/nested/moved.txt"), "utf8"),
      "created and dirty",
    );
  });
  await test("EDIT-16 deletion is confirmed and recoverable", async () => {
    await view("Explorer");
    const nested = explorer()
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", { hasText: /^nested$/ }),
      });
    if ((await nested.getAttribute("aria-expanded")) !== "true")
      await nested.click();
    await explorer()
      .getByRole("treeitem")
      .filter({ hasText: "moved.txt" })
      .click({ button: "right" });
    await page
      .getByRole("menuitem", { name: "Delete to ASF recovery", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Confirm deletion", exact: true })
      .click();
    await explorer()
      .getByRole("button", { name: "Undo deletion", exact: true })
      .waitFor();
    await assert.rejects(fs.access(path.join(a, "src/nested/moved.txt")));
    await explorer()
      .getByRole("button", { name: "Undo deletion", exact: true })
      .click();
    await eventually(async () => {
      await fs.access(path.join(a, "src/nested/moved.txt"));
      return true;
    }, "restored file");
  });
  await test("EDIT-17/18 actual filename and content search navigation", async () => {
    await view("Search");
    const search = page.locator(".files-search:visible");
    await search.getByLabel("Find file", { exact: true }).fill("module.ts");
    await search.getByRole("button", { name: "Search", exact: true }).click();
    await search.locator(".files-search-hit").first().waitFor();
    assert.match(await search.innerText(), /src\/nested\/module.ts/);
    await search.getByLabel("Search mode").selectOption("content");
    await search.getByLabel("Search text", { exact: true }).fill("needle");
    await search.getByRole("button", { name: "Search", exact: true }).click();
    await search.locator(".files-search-hit").first().waitFor();
    assert.match(await search.innerText(), /module.ts:1:/);
    await shot("05-real-search.png");
    await search.locator(".files-search-hit").first().click();
    await page
      .getByRole("textbox", { name: "Source editor src/nested/module.ts" })
      .last()
      .waitFor();
  });
  await test("EDIT-20 real HEAD and buffer diff is read only", async () => {
    await openPath("src/main.rs");
    await editor()
      .getByRole("button", { name: "Compare HEAD", exact: true })
      .click();
    await eventually(
      async () =>
        (await page.locator(".files-merge:visible").innerText()).includes(
          "Project A",
        ),
      "Git HEAD diff",
    );
    assert.equal(
      await page.locator(".files-merge:visible [contenteditable=true]").count(),
      0,
    );
    const original = page.locator(".files-merge:visible").getByRole("textbox", {
      name: "Compared source version",
      exact: true,
    });
    await original.focus();
    assert.equal(
      await original.evaluate((node) => document.activeElement === node),
      true,
    );
    await original.press("ArrowDown");
    await shot("06-git-head-diff.png");
  });
  await test("Local editor find, replace, undo and redo use the shared real document", async () => {
    await openPath("src/nested/module.ts");
    const area = editor("module.ts");
    await area
      .getByRole("button", { name: "Find / replace", exact: true })
      .click();
    await area.locator('.cm-search input[name="search"]').fill("needle");
    await area.locator('.cm-search input[name="replace"]').fill("replaced");
    await area
      .getByRole("button", { name: "replace all", exact: true })
      .click();
    const source = page.getByRole("textbox", {
      name: "Source editor src/nested/module.ts",
      exact: true,
    });
    assert.match(await source.innerText(), /replaced/);
    assert.match(
      await fs.readFile(path.join(a, "src/nested/module.ts"), "utf8"),
      /needle/,
    );
    await source.click();
    await source.press("ControlOrMeta+z");
    assert.match(await source.innerText(), /needle/);
    await source.press(
      process.platform === "darwin" ? "Meta+Shift+z" : "Control+y",
    );
    assert.match(await source.innerText(), /replaced/);
    await save("module.ts");
    assert.equal(
      await fs.readFile(path.join(a, "src/nested/module.ts"), "utf8"),
      "export const value = 'replaced';\n",
    );
    await shot("13-editor-find-replace.png");
    await page.addScriptTag({ path: axePath });
    const audit = await page.evaluate(async () =>
      window.axe.run(
        { include: [[".files-editor:has(.cm-search)"]] },
        {
          runOnly: {
            type: "tag",
            values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
          },
        },
      ),
    );
    results.editorAccessibility = {
      violations: audit.violations.map((item) => ({
        id: item.id,
        impact: item.impact,
        nodes: item.nodes.map((node) => node.target),
      })),
      incomplete: audit.incomplete.map((item) => ({
        id: item.id,
        nodes: item.nodes.map((node) => node.target),
      })),
    };
    assert.equal(
      audit.violations.length,
      0,
      JSON.stringify(results.editorAccessibility),
    );
  });
  await test("EDIT-21 binary file is metadata-only", async () => {
    await view("Explorer");
    await explorer()
      .getByRole("treeitem")
      .filter({ hasText: "binary.bin" })
      .click();
    await page
      .getByRole("heading", { name: "File metadata", exact: true })
      .waitFor();
    assert.match(
      await page.locator(".files-editor:visible").last().innerText(),
      /binary/,
    );
    for (const label of ["Compare disk", "Compare base", "Compare HEAD"])
      assert.equal(
        await editor("binary.bin")
          .getByRole("button", { name: label, exact: true })
          .isEnabled(),
        false,
      );
  });
  await test("EDIT-22 oversized text loads a read-only bounded preview", async () => {
    await openPath("large.txt");
    assert.match(await editor("large.txt").innerText(), /bounded preview/);
    assert.equal(
      await editor("large.txt")
        .getByRole("button", { name: "Compare base", exact: true })
        .isEnabled(),
      false,
    );
    assert.equal(
      await page
        .getByRole("textbox", { name: "Source editor large.txt" })
        .getAttribute("contenteditable"),
      "false",
    );
  });
  await test("EDIT-24 managed agent worktree is a distinct read-only root", async () => {
    await view("Explorer");
    await explorer().getByLabel("Filesystem root").selectOption(wr);
    await openPath("src/main.rs");
    assert.match(
      await page
        .locator(`.files-editor[data-root-id="${wr}"]:visible`)
        .innerText(),
      /Managed roots are available for inspection\. Manual writes are disabled to preserve execution and review evidence\./,
    );
    assert.match(
      await page
        .locator(`.files-editor[data-root-id="${wr}"]:visible`)
        .getByRole("textbox", { name: "Source editor src/main.rs" })
        .innerText(),
      /Project A/,
    );
    assert.equal(
      await page
        .locator(`.files-editor[data-root-id="${wr}"]:visible`)
        .getByRole("button", { name: "Save", exact: true })
        .isEnabled(),
      false,
    );
    const source = page
      .locator(`.files-editor[data-root-id="${wr}"]:visible`)
      .getByRole("textbox", { name: "Source editor src/main.rs" });
    await source.focus();
    assert.equal(
      await source.evaluate((node) => document.activeElement === node),
      true,
    );
    await source.press("ArrowDown");
    await shot("07-agent-worktree-read-only.png");
  });
  await test("EDIT-25 protected roots and worktree mutation are rejected by Rust", async () => {
    const file = { project_id: aid, root_id: ar, path: ".factory/db.sqlite3" };
    assert.equal(
      (
        await mutate(
          `/projects/${aid}/files/entries`,
          "DELETE",
          { file, expected_revision: "v_fake", confirmed: true },
          true,
        )
      ).status,
      403,
    );
    const snapshot = await read(
      `/projects/${aid}/files/content?root_id=${wr}&path=src/main.rs`,
    );
    assert.equal(
      (
        await mutate(
          `/projects/${aid}/files/content`,
          "PUT",
          {
            file: snapshot.file,
            expected_revision: snapshot.revision,
            content: "bad",
          },
          true,
        )
      ).status,
      403,
    );
  });
  await test("EDIT-08 backend restart preserves editor layout and recoverable state", async () => {
    await idle();
    await stop();
    await start();
    await auth();
    await page.reload();
    await page.locator(".ws-status").waitFor();
    await selectProject(aid);
    await mutate(`/projects/${aid}/open`, "POST", {});
    await idle();
    const restoredId = await page.evaluate(() =>
      localStorage.getItem("asf.v2.workspace.last"),
    );
    const restored = await read(`/workspaces/${restoredId}`);
    const recovery = structuredClone(restored);
    recovery.revision = Math.max(0, restored.revision - 1);
    recovery.document.preferences.theme =
      restored.document.preferences.theme === "light" ? "dark" : "light";
    await page.evaluate(
      (value) =>
        sessionStorage.setItem(
          "asf.v2.workspace.recovery",
          JSON.stringify(value),
        ),
      recovery,
    );
    await page.reload();
    await page.locator(".files-editor:visible").first().waitFor();
    await preserveRecoveredLayout();
    assert.equal(results.layoutRecoveries?.length, 1);
    await idle();
    assert.equal(results.errors.length, 0, results.errors.join("\n"));
  });
  await test("Light theme and narrow desktop remain coherent", async () => {
    await page.getByLabel("Theme", { exact: true }).selectOption("light");
    await page.setViewportSize({ width: 1280, height: 800 });
    await shot("08-light-workspace.png");
    await page.setViewportSize({ width: 1024, height: 768 });
    await shot("09-narrow-workspace.png");
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth > innerWidth,
      ),
      false,
    );
  });
  await test("EDIT-26 native write refusal preserves the dirty buffer and close confirmation", async () => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await view("Explorer");
    await explorer().getByLabel("Filesystem root").selectOption(ar);
    const target = path.join(a, "src/refused.txt");
    await fs.writeFile(target, "original stays");
    await explorer().getByRole("button", { name: "Refresh explorer" }).click();
    await openPath("src/refused.txt");
    await edit("src/refused.txt", "must remain a draft");
    await page.route("**/api/projects/*/files/content", async (route) => {
      if (route.request().method() === "PUT") {
        await fs.chmod(target, 0o444);
        await route.continue();
      } else await route.continue();
    });
    try {
      const rejected = page.waitForResponse(
        (response) =>
          response.request().method() === "PUT" &&
          response.url().endsWith("/files/content"),
      );
      await editor("refused.txt")
        .getByRole("button", { name: "Save", exact: true })
        .click();
      assert.ok((await rejected).status() >= 400);
      await eventually(async () =>
        (await editor("refused.txt").innerText()).includes(
          "File operation failed",
        ),
      );
      assert.equal(await fs.readFile(target, "utf8"), "original stays");
      assert.match(
        await page
          .getByRole("textbox", { name: "Source editor src/refused.txt" })
          .innerText(),
        /must remain a draft/,
      );
    } finally {
      await page.unroute("**/api/projects/*/files/content");
      await fs.chmod(target, 0o666);
    }
    await editor("refused.txt")
      .getByRole("button", { name: "Save all", exact: true })
      .focus();
    await page.keyboard.press("Alt+Shift+W");
    const dialog = page.getByRole("dialog", {
      name: "Unsaved changes",
      exact: true,
    });
    await dialog.waitFor();
    assert.equal(
      await dialog.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return [...element.querySelectorAll("footer button")].every(
          (button) => {
            const rect = button.getBoundingClientRect();
            return rect.left >= bounds.left && rect.right <= bounds.right;
          },
        );
      }),
      true,
    );
    await shot("14-dirty-close-confirmation.png");
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor src/refused.txt" })
        .innerText(),
      /must remain a draft/,
    );
    await page.keyboard.press("Alt+Shift+W");
    await dialog
      .getByRole("button", { name: "Discard changes and close", exact: true })
      .click();
    await dialog.waitFor({ state: "hidden" });
    assert.equal(await fs.readFile(target, "utf8"), "original stays");
  });
  await test("Copied browser-window ownership and draft CAS preserve both versions", async () => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await view("Explorer");
    await explorer().getByLabel("Filesystem root").selectOption(ar);
    await fs.writeFile(path.join(a, "src/window.txt"), "base window content");
    await explorer().getByRole("button", { name: "Refresh explorer" }).click();
    await openPath("src/window.txt");
    await edit("src/window.txt", "window A initial");
    await eventually(async () =>
      (await editor("window.txt").innerText()).includes("Draft backed up"),
    );
    await idle();
    const owner = await page.evaluate(() =>
        sessionStorage.getItem("asf.v2.editor.window"),
      ),
      second = await context.newPage();
    await second.addInitScript(
      (owner) => sessionStorage.setItem("asf.v2.editor.window", owner),
      owner,
    );
    await second.goto(`${base}/workspace.html`);
    const secondSource = second.getByRole("textbox", {
      name: "Source editor src/window.txt",
      exact: true,
    });
    await secondSource.waitFor();
    await eventually(
      async () =>
        (await second.evaluate(() =>
          sessionStorage.getItem("asf.v2.editor.window"),
        )) !== owner,
      "distinct duplicated-tab owner",
    );
    await secondSource.click();
    await secondSource.press("ControlOrMeta+a");
    await secondSource.pressSequentially("window B retained");
    await eventually(async () =>
      (
        await second
          .locator(".files-editor:visible")
          .filter({ has: secondSource })
          .innerText()
      ).includes("Draft backed up"),
    );
    await edit("src/window.txt", "window A newer");
    await eventually(
      async () =>
        (await editor("window.txt").innerText()).includes(
          "Save independent draft",
        ),
      "recovery CAS conflict",
    );
    const retained = await page.evaluate(async () => {
      const database = await new Promise((resolve, reject) => {
        const request = indexedDB.open("asf-editor-drafts", 1);
        request.onsuccess = () => resolve(request.result);
        request.onerror = reject;
      });
      return new Promise((resolve) => {
        const read = database
          .transaction("drafts")
          .objectStore("drafts")
          .getAll();
        read.onsuccess = () => {
          resolve(
            read.result
              .filter((item) => item.document.file.path === "src/window.txt")
              .map((item) => ({
                owner: item.owner,
                content: item.document.content,
              })),
          );
          database.close();
        };
      });
    });
    assert.ok(new Set(retained.map((item) => item.owner)).size >= 2);
    assert.ok(retained.some((item) => item.content === "window A newer"));
    assert.ok(retained.some((item) => item.content === "window B retained"));
    await editor("window.txt")
      .getByRole("button", { name: "Save independent draft", exact: true })
      .click();
    await eventually(async () =>
      (await editor("window.txt").innerText()).includes("Draft backed up"),
    );
    await save("window.txt");
    assert.equal(
      await fs.readFile(path.join(a, "src/window.txt"), "utf8"),
      "window A newer",
    );
    await eventually(
      async () =>
        (
          await second
            .locator(".files-editor:visible")
            .filter({ has: secondSource })
            .innerText()
        ).includes("Disk changed while"),
      "other-window disk conflict",
    );
    assert.match(await secondSource.innerText(), /window B retained/);
    await second.close();
    await eventually(async () => {
      const copy = page.getByRole("button", {
        name: "Save independent copy",
        exact: true,
      });
      if (await copy.isVisible()) await copy.click();
      return (await page.locator(".ws-status").innerText()).includes(
        "Workspace saved",
      );
    }, "independent layout recovery");
    await shot("12-multi-window-recovery.png");
  });
  await test("Keyboard navigation, disabled writes and component accessibility", async () => {
    await view("Explorer");
    await explorer().getByLabel("Filesystem root").selectOption(ar);
    await eventually(
      async () =>
        (await explorer().getByRole("tree").getAttribute("aria-busy")) ===
        "false",
    );
    const tree = explorer().getByRole("tree");
    await tree.focus();
    await tree.press("Home");
    await tree.press("ArrowRight");
    await tree.press("ArrowDown");
    assert.ok(await tree.getAttribute("aria-activedescendant"));
    await tree.press("Shift+F10");
    await page.getByRole("menu", { name: "File actions" }).waitFor();
    await page.keyboard.press("Escape");
    await page.addScriptTag({ path: axePath });
    const audit = await page.evaluate(async () =>
      window.axe.run(
        { include: [[".files-explorer"]] },
        {
          runOnly: {
            type: "tag",
            values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
          },
        },
      ),
    );
    results.accessibility = {
      violations: audit.violations.map((item) => ({
        id: item.id,
        impact: item.impact,
        nodes: item.nodes.map((node) => node.target),
      })),
      incomplete: audit.incomplete.map((item) => ({
        id: item.id,
        nodes: item.nodes.map((node) => node.target),
      })),
    };
    assert.equal(
      audit.violations.length,
      0,
      JSON.stringify(results.accessibility),
    );
  });
  await test("EDIT-30 V1 dashboard still opens against the same backend", async () => {
    const legacy = await context.newPage();
    await legacy.goto(`${base}/?project=${aid}`);
    await legacy.getByText("Agent Graph", { exact: true }).first().waitFor();
    await legacy.close();
  });
  await test("Measured real-file APIs, bounded search and large explorer", async () => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await view("Explorer");
    await explorer().getByLabel("Filesystem root").selectOption(ar);
    await fs.mkdir(path.join(a, "bench"));
    for (let i = 0; i < 30; i++)
      await fs.writeFile(
        path.join(a, "bench", `file-${String(i).padStart(3, "0")}.ts`),
        `export const value${i} = 'unique_benchmark';\n`.repeat(20),
      );
    for (const [name, size] of [
      ["small.txt", 256],
      ["medium.txt", 262144],
      ["large-supported.txt", 786432],
    ])
      await fs.writeFile(path.join(a, "bench", name), "x".repeat(size));
    const csrf = await auth();
    async function measure(name, url, options = {}) {
      const start = performance.now();
      const response = await context.request.fetch(
        `${base}/api/projects/${aid}/${url}`,
        { ...options, headers: { Origin: base, "X-Factory-CSRF": csrf } },
      );
      const text = await response.text();
      assert.ok(response.ok(), text);
      const value = JSON.parse(text);
      results.measurements.push({
        name,
        ms: performance.now() - start,
        responseBytes: Buffer.byteLength(text),
        complete: value.complete,
        reason: value.reason,
        scanned: value.scanned,
        bytesRead: value.bytes_read,
      });
      return value;
    }
    for (const name of ["small.txt", "medium.txt", "large-supported.txt"])
      for (let i = 0; i < 5; i++)
        await measure(
          `read-${name}`,
          `files/content?root_id=${ar}&path=bench/${name}`,
        );
    const saveFile = { project_id: aid, root_id: ar, path: "bench/small.txt" };
    let current = await read(
      `/projects/${aid}/files/content?root_id=${ar}&path=bench/small.txt`,
    );
    for (let i = 0; i < 5; i++)
      current = await measure("save-small", "files/content", {
        method: "PUT",
        data: {
          file: saveFile,
          expected_revision: current.revision,
          content: `saved ${i}`,
        },
      });
    const searchRequest = () => ({
      id: `q_${crypto.randomUUID().replaceAll("-", "")}`,
      root_id: ar,
      query: "unique_benchmark",
      content: true,
      case_sensitive: true,
      regex: false,
      include_hidden: false,
      path_filter: "bench/",
    });
    await measure("search-medium", "files/search", {
      method: "POST",
      data: searchRequest(),
    });
    await view("Explorer");
    await explorer().getByRole("button", { name: "Refresh explorer" }).click();
    await eventually(
      async () =>
        (await explorer().getByRole("tree").getAttribute("aria-busy")) ===
        "false",
    );
    for (const name of ["small.txt", "medium.txt", "large-supported.txt"]) {
      const start = performance.now();
      await openPath(`bench/${name}`);
      results.measurements.push({
        name: `open-ui-${name}`,
        ms: performance.now() - start,
      });
    }
    await fs.mkdir(path.join(a, "bulk"));
    for (let offset = 0; offset < 5000; offset += 50)
      await Promise.all(
        Array.from({ length: 50 }, (_, index) =>
          fs.writeFile(
            path.join(
              a,
              "bulk",
              `bulk-file-${String(offset + index).padStart(5, "0")}.txt`,
            ),
            "bounded content\n",
          ),
        ),
      );
    results.fixtureSizes = {
      bulkFiles: 5000,
      editorTabs: 30,
      mediumBytes: 262144,
      largeSupportedBytes: 786432,
    };
    await measure("search-large", "files/search", {
      method: "POST",
      data: { ...searchRequest(), query: "absent content", path_filter: "" },
    });
    const listing = await measure(
      "directory-5000",
      "files/tree?root_id=" + ar + "&path=bulk&hidden=false",
    );
    assert.equal(listing.entries.length, 200);
    assert.ok(listing.next);
    await view("Explorer");
    await explorer().getByRole("button", { name: "Refresh explorer" }).click();
    await eventually(
      async () =>
        (await explorer().getByRole("tree").getAttribute("aria-busy")) ===
        "false",
    );
    const bulk = explorer()
      .getByRole("treeitem")
      .filter({ has: page.locator(".files-tree-name", { hasText: /^bulk$/ }) });
    await explorer()
      .getByRole("tree")
      .evaluate((element) => {
        element.scrollTop = element.scrollHeight;
      });
    const start = performance.now();
    await bulk.click();
    await explorer()
      .getByRole("treeitem")
      .filter({
        has: page.locator(".files-tree-name", {
          hasText: /^bulk-file-00000\.txt$/,
        }),
      })
      .waitFor();
    await eventually(
      async () =>
        (await explorer().getByRole("tree").getAttribute("aria-busy")) ===
        "false",
    );
    results.measurements.push({
      name: "expand-5000-ui",
      ms: performance.now() - start,
    });
    assert.ok((await explorer().getByRole("treeitem").count()) < 100);
    await shot("10-large-lazy-explorer.png");
    await view("Search");
    const search = page.locator(".files-search:visible");
    await search.getByLabel("Search mode").selectOption("files");
    await search.getByLabel("Find file", { exact: true }).fill("file-");
    await search.getByLabel("Path contains", { exact: true }).fill("bench/");
    await search.getByRole("button", { name: "Search", exact: true }).click();
    await search.locator(".files-search-hit").first().waitFor();
    const opened = performance.now();
    for (let i = 0; i < 30; i++) {
      await view("Search");
      await search
        .locator(".files-search-hit")
        .filter({ hasText: `bench/file-${String(i).padStart(3, "0")}.ts` })
        .click();
      await page
        .getByRole("textbox", {
          name: `Source editor bench/file-${String(i).padStart(3, "0")}.ts`,
          exact: true,
        })
        .waitFor();
    }
    results.measurements.push({
      name: "open-30-tabs-ui",
      ms: performance.now() - opened,
    });
    await page
      .getByRole("textbox", {
        name: "Source editor bench/file-029.ts",
        exact: true,
      })
      .focus();
    const visibleIds = () =>
      page.locator(".files-editor:visible").evaluateAll((elements) =>
        elements
          .map((element) => element.getAttribute("data-document-id"))
          .sort()
          .join("|"),
      );
    for (let i = 0; i < 5; i++) {
      const before = await visibleIds();
      const start = performance.now();
      await page.keyboard.press("Alt+PageUp");
      await eventually(
        async () => (await visibleIds()) !== before,
        "actual editor tab switch",
      );
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );
      results.measurements.push({
        name: "switch-tab-ui",
        ms: performance.now() - start,
      });
    }
    await idle();
    const before = await page.locator(".files-editor").count();
    const restoring = performance.now();
    await page.reload();
    await eventually(
      async () => (await page.locator(".files-editor").count()) === before,
      "30-tab document restoration",
      60000,
    );
    results.measurements.push({
      name: "restore-many-tabs-ui",
      tabs: before,
      ms: performance.now() - restoring,
    });
    const cdp = await context.newCDPSession(page);
    await cdp.send("Performance.enable");
    const metrics = await cdp.send("Performance.getMetrics");
    results.browserMemory = metrics.metrics.filter((value) =>
      [
        "JSHeapUsedSize",
        "JSHeapTotalSize",
        "Nodes",
        "Documents",
        "Frames",
      ].includes(value.name),
    );
    await cdp.detach();
    await shot("11-many-editor-tabs.png");
  });
  assert.equal(results.errors.length, 0);
  assert.equal(results.requests.length, 0);
} catch (error) {
  results.failure = error.stack;
  process.exitCode = 1;
} finally {
  await context?.close();
  await browser?.close();
  await stop();
  results.finished = new Date().toISOString();
  await fs.writeFile(
    path.join(output, "editor-browser-results.json"),
    JSON.stringify(results, null, 2),
  );
  console.log(
    JSON.stringify(
      {
        tests: results.tests,
        failure: results.failure,
        screenshots: results.screenshots,
      },
      null,
      2,
    ),
  );
}
