import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs/promises";
import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import os from "node:os";
import { createHash } from "node:crypto";
const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const output = path.resolve(root, process.env.ASF_P06_OUTPUT_DIR ?? ".factory/v2/p06"),
  shots = path.join(output, "screenshots");
await fs.mkdir(shots, { recursive: true });
const binary =
  process.env.ASF_P06_BINARY ??
  path.join(
    output,
    "run",
    process.platform === "win32" ? "factory.exe" : "factory",
  );
const profiles = JSON.parse(
  (
    await fs.readFile(
      process.env.ASF_IDE_TEST_TOOLS ??
        path.join(output, "tools/profiles.json"),
      "utf8",
    )
  ).replace(/^\uFEFF/, ""),
);
const fixture = await fs.mkdtemp(path.join(output, "browser-projects-"));
const registry = path.join(
  await fs.mkdtemp(path.join(os.tmpdir(), "asf-p06-registry-")),
  "projects.sqlite3",
);
const result = {
  schema: 1,
  sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
    windowsHide: true,
  }).stdout.trim(),
  scriptSha256: createHash("sha256")
    .update(await fs.readFile(fileURLToPath(import.meta.url)))
    .digest("hex"),
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
  processes: [],
  requests: [],
};
const run = (program, args, cwd = root) => {
  const outcome = spawnSync(program, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  assert.equal(outcome.status, 0, outcome.stderr);
  return outcome.stdout;
};
for (const name of ["Project A", "Project B"]) {
  const folder = path.join(fixture, name);
  await fs.mkdir(folder);
  run("git", ["init", "-q", "-b", "main"], folder);
  run(binary, ["init"], folder);
  await fs.writeFile(
    path.join(folder, "main.ts"),
    "export function add(a: number, b: number) { return a + b; }\nconst value: string = add(1, 2);\n",
  );
  await fs.writeFile(
    path.join(folder, "debug.py"),
    "value = 40\nvalue += 2\nprint(value)\n",
  );
  await fs.writeFile(
    path.join(folder, "main.py"),
    "def add(a: int, b: int) -> int:\n    return a + b\n\nvalue: str = add(1, 2)\n",
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
let child, browser, context, page;
async function eventually(action, message = "condition", timeout = 30000) {
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
      cwd: path.join(fixture, "Project A"),
      windowsHide: true,
      stdio: ["ignore", log.fd, log.fd],
    },
  );
  result.processes.push(child.pid);
  await log.close();
  await eventually(async () => {
    try {
      return (await fetch(`${base}/api/health`)).ok;
    } catch {
      return false;
    }
  }, "server start");
}
async function stop() {
  if (child?.exitCode === null)
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
async function mutate(project, suffix, data = {}, allowError = false) {
  const csrf = await auth();
  const response = await context.request.post(
    `${base}/api/projects/${project}/${suffix}`,
    {
      headers: { Origin: base, "X-Factory-CSRF": csrf },
      data,
      timeout: 125000,
    },
  );
  if (!allowError)
    assert.ok(response.ok(), `${suffix}: ${await response.text()}`);
  return response;
}
async function get(project, suffix) {
  const response = await context.request.get(
    `${base}/api/projects/${project}/${suffix}`,
  );
  assert.ok(response.ok(), await response.text());
  return response.json();
}
async function view(name) {
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}
async function selectProject(id) {
  await page.getByLabel("Open registered project").selectOption(id);
  await page.locator(`#project-${id}[aria-selected=true]`).waitFor();
}
async function openFile(name) {
  await view("Explorer");
  const row = page
    .locator(".files-explorer:visible")
    .getByRole("treeitem")
    .filter({
      has: page.locator(".files-tree-name", {
        hasText: new RegExp(`^${name.replaceAll(".", "\\.")}$`),
      }),
    })
    .first();
  await row.click();
  await page
    .getByRole("textbox", { name: `Source editor ${name}`, exact: true })
    .last()
    .waitFor();
}
const editor = () => page.locator(".files-editor:visible").first();
async function shot(name) {
  await page.screenshot({ path: path.join(shots, name) });
  result.screenshots.push(name);
}
async function test(name, action) {
  const started = performance.now();
  try {
    await action();
    result.tests.push({
      name,
      status: "passed",
      duration_ms: performance.now() - started,
    });
    console.log(`PASS ${name}`);
  } catch (error) {
    result.tests.push({ name, status: "failed", error: error.message });
    await page
      ?.screenshot({ path: path.join(shots, "failure.png") })
      .catch(() => {});
    throw error;
  } finally {
    await fs.writeFile(
      path.join(output, "browser-results.json"),
      JSON.stringify(result, null, 2),
    );
  }
}
let a, b, ar, br, lspId;
try {
  await start();
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
  a = registered[0].project_id;
  const csrf = await auth();
  const response = await context.request.post(`${base}/api/projects/register`, {
    headers: { Origin: base, "X-Factory-CSRF": csrf },
    data: { path: path.join(fixture, "Project B"), trust_project: true },
  });
  assert.ok(response.ok(), await response.text());
  b = (await response.json()).project_id;
  await mutate(b, "open");
  ar = (await get(a, "files/roots")).roots.find((root) => !root.read_only).id;
  br = (await get(b, "files/roots")).roots.find((root) => !root.read_only).id;
  result.projects = { a, b, ar, br };
  for (const project of [a, b])
    for (const profile of profiles)
      await mutate(project, "ide/profiles", { profile, confirmed: true });
  await mutate(b, "ide/configurations", {
    configuration: {
      id: "python-main",
      name: "Python fixture",
      profile_id: "debugpy",
      root_id: br,
      program: { project_id: b, root_id: br, path: "debug.py" },
      arguments: { stopOnEntry: false },
    },
    confirmed: true,
  });
  page = await context.newPage();
  page.setDefaultTimeout(18000);
  page.on("pageerror", (error) => result.errors.push(error.message));
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== base)
      result.requests.push(request.url());
  });
  await page.goto(`${base}/workspace.html`);
  await selectProject(a);
  await test("LSP-01 browser launches real TypeScript server and diagnostics", async () => {
    await openFile("main.ts");
    await editor()
      .getByRole("button", { name: "Start language server", exact: true })
      .click();
    await eventually(async () => {
      const state = await get(a, "ide/state");
      lspId = state.servers.find(
        (server) => server.profile === "typescript",
      )?.id;
      return !!lspId;
    }, "TypeScript launch");
    await eventually(async () => {
      const events = await get(a, `ide/lsp/${lspId}/events?cursor=0`);
      return events.events.some(
        (event) =>
          event.message.method === "textDocument/publishDiagnostics" &&
          event.message.params.diagnostics.length > 0,
      );
    }, "real TypeScript diagnostics");
    await page.locator(".cm-lintRange-error").first().waitFor();
    await shot("01-typescript-diagnostics-dark.png");
  });
  await test("LSP-02 unsaved buffer completion and diagnostics", async () => {
    const source = page
      .getByRole("textbox", { name: "Source editor main.ts", exact: true })
      .last();
    await source.click();
    await source.press("ControlOrMeta+End");
    await source.pressSequentially("\nad");
    await source.press("Control+Space");
    await page.locator(".cm-tooltip-autocomplete").waitFor();
    await shot("02-completion.png");
    await source.press("Enter");
    assert.match(await source.innerText(), /\nadd\s*$/);
    await source.press("ControlOrMeta+a");
    await source.press("Backspace");
    await source.pressSequentially(
      "export function add(a: number, b: number) { return a + b; }\nconst value: number = add(1, 2);\n",
    );
    await eventually(async () => {
      const events = await get(a, `ide/lsp/${lspId}/events?cursor=0`);
      return events.events.some(
        (event) =>
          event.message.method === "textDocument/publishDiagnostics" &&
          event.message.params.diagnostics.every(
            (diagnostic) => diagnostic.severity !== 1,
          ),
      );
    }, "unsaved diagnostics refresh");
    assert.match(
      await fs.readFile(path.join(fixture, "Project A/main.ts"), "utf8"),
      /value: string/,
    );
  });
  await test("LSP-03 hover definition and real save", async () => {
    const source = page
      .getByRole("textbox", { name: "Source editor main.ts", exact: true })
      .last();
    await source.getByText("add", { exact: true }).first().hover();
    await page.locator(".cm-tooltip-hover").first().waitFor();
    assert.match(
      await page.locator(".cm-tooltip-hover").first().innerText(),
      /number/,
    );
    await shot("03a-real-hover.png");
    await source.click();
    await source.press("ControlOrMeta+Home");
    await source.press("ArrowDown");
    await source.press("Home");
    for (
      let i = 0;
      i < "const value: number = add(1, 2);".indexOf("add") + 1;
      i++
    )
      await source.press("ArrowRight");
    await editor()
      .getByRole("button", { name: "Definition", exact: true })
      .click();
    await eventually(
      async () =>
        (await source
          .locator("xpath=..")
          .locator(".cm-activeLineGutter")
          .first()
          .textContent()) === "1",
      "definition moved to the declaration",
    );
    await shot("03-definition-navigation.png");
    await editor().getByRole("button", { name: "Save", exact: true }).click();
    await eventually(
      async () =>
        /value: number/.test(
          await fs.readFile(path.join(fixture, "Project A/main.ts"), "utf8"),
        ),
      "real save",
    );
  });
  await test("LSP references symbols and reviewed rename", async () => {
    await editor()
      .getByRole("button", { name: "References", exact: true })
      .click();
    const summary = editor().getByText(
      "Symbols, code actions and extension commands",
      { exact: true },
    );
    if (
      !(await editor()
        .getByRole("button", { name: "Document symbols", exact: true })
        .isVisible())
    )
      await summary.click();
    await eventually(
      () =>
        editor()
          .locator(".ide-list li")
          .count()
          .then((count) => count >= 2),
      "real symbol references",
    );
    await editor()
      .getByRole("button", { name: "Document symbols", exact: true })
      .click();
    await eventually(
      () =>
        editor()
          .locator(".ide-list li")
          .count()
          .then((count) => count > 0),
      "real document symbols",
    );
    await editor()
      .getByRole("button", { name: "Rename symbol", exact: true })
      .click();
    await page.getByLabel("New symbol name", { exact: true }).fill("sum");
    await page
      .getByRole("button", { name: "Request rename", exact: true })
      .click();
    const review = page.getByRole("dialog", {
      name: "Review language workspace edit",
    });
    await review.waitFor();
    await review
      .getByRole("button", { name: "Apply reviewed edits", exact: true })
      .click();
    await review.waitFor({ state: "hidden" });
    assert.match(
      await page
        .getByRole("textbox", { name: "Source editor main.ts", exact: true })
        .last()
        .innerText(),
      /function sum/,
    );
  });
  await test("LSP formatting uses reviewed version-aware edits", async () => {
    await editor().getByRole("button", { name: "Format", exact: true }).click();
    const dialog = page.getByRole("dialog", {
      name: "Review language workspace edit",
    });
    await dialog.waitFor();
    await dialog.getByRole("button", { name: "Apply reviewed edits" }).click();
    await dialog.waitFor({ state: "hidden" });
  });
  await test("LSP signature help and selectable reviewed code action", async () => {
    const source = page
      .getByRole("textbox", { name: "Source editor main.ts", exact: true })
      .last();
    const previous = await source.innerText();
    async function replace(text) {
      await source.click();
      await source.press("ControlOrMeta+a");
      await source.press("Backspace");
      await source.pressSequentially(text);
    }
    await replace(
      "export function add(a: number, b: number) { return a + b; }\nadd(",
    );
    await source.press("ControlOrMeta+Shift+Space");
    await page.locator(".cm-lsp-signature-tooltip").waitFor();
    assert.match(
      await page.locator(".cm-lsp-signature-tooltip").innerText(),
      /a: number/,
    );
    await shot("03b-signature-help.png");
    await source.press("Escape");
    await replace("export const value = 42;\nconsole.log(vlaue);\n");
    await page.locator(".cm-lintRange-error").first().waitFor();
    await source.press("ControlOrMeta+Home");
    await source.press("ArrowDown");
    await source.press("Home");
    for (let i = 0; i < "console.log(vlaue);".indexOf("vlaue") + 1; i++)
      await source.press("ArrowRight");
    await editor()
      .getByRole("button", { name: "Code actions", exact: true })
      .click();
    const choice = page.getByRole("dialog", { name: "Language code actions" });
    await choice.waitFor();
    await choice.locator(".ide-list button:not(:disabled)").first().click();
    const review = page.getByRole("dialog", {
      name: "Review language workspace edit",
    });
    await review.waitFor();
    await shot("03c-code-action-review.png");
    await review
      .getByRole("button", { name: "Apply reviewed edits", exact: true })
      .click();
    await review.waitFor({ state: "hidden" });
    assert.match(await source.innerText(), /console.log\(value\)/);
    await replace(previous);
  });
  await test("Project-specific LSP access refuses other project", async () => {
    const response = await mutate(
      b,
      `ide/lsp/${lspId}/request`,
      {
        method: "textDocument/hover",
        params: { textDocument: { uri: "file:///outside.ts" } },
        confirmed: false,
      },
      true,
    );
    assert.equal(response.status(), 404);
  });
  await test("Independent browser windows do not share unsaved LSP documents", async () => {
    const firstEditor = page
      .getByRole("textbox", { name: "Source editor main.ts", exact: true })
      .last();
    const visibleText = (
      await firstEditor.locator(".cm-line").allTextContents()
    ).join("\n");
    const visibleHash = createHash("sha256").update(visibleText).digest("hex");
    const before = await eventually(async () => {
      const current = (await get(a, "ide/state")).servers.find(
        (server) => server.id === lspId,
      );
      return current?.documents.some(
        (doc) => doc.content_sha256 === visibleHash,
      )
        ? current
        : null;
    }, "first window synchronization before isolation measurement");
    const owner = await page.evaluate(() =>
      sessionStorage.getItem("asf.v2.editor.window"),
    );
    assert.ok(owner);
    assert.ok(before.client_id.endsWith(`.${owner}`));
    const other = await context.newPage();
    try {
      await other.addInitScript(
        (value) => sessionStorage.setItem("asf.v2.editor.window", value),
        owner,
      );
      await other.goto(`${base}/workspace.html`);
      const second = other
        .getByRole("textbox", { name: "Source editor main.ts", exact: true })
        .last();
      await second.waitFor();
      const panel = other
        .locator(".files-editor")
        .filter({ has: second })
        .last();
      await panel
        .getByRole("button", { name: "Start language server", exact: true })
        .click();
      const launched = await eventually(async () => {
        const state = await get(a, "ide/state");
        return state.servers.find(
          (server) => server.alive && server.id !== lspId,
        );
      }, "independent window server");
      assert.notEqual(launched.client_id, before.client_id);
      assert.notEqual(launched.pid, before.pid);
      await second.click();
      await second.press("ControlOrMeta+End");
      await second.pressSequentially("\n// independent language buffer\n");
      await eventually(async () => {
        const current = (await get(a, "ide/state")).servers.find(
          (server) => server.id === launched.id,
        );
        return current.documents.some(
          (doc) =>
            !before.documents.some(
              (original) => doc.content_sha256 === original.content_sha256,
            ),
        );
      }, "independent language text synchronization");
      const original = (await get(a, "ide/state")).servers.find(
        (server) => server.id === lspId,
      );
      assert.deepEqual(original.documents, before.documents);
      assert.equal(
        (await firstEditor.locator(".cm-line").allTextContents()).join("\n"),
        visibleText,
      );
      await mutate(a, `ide/lsp/${launched.id}/stop`);
    } finally {
      await other.close();
    }
    const fork = page.getByRole("button", {
      name: "Save independent copy",
      exact: true,
    });
    if (await fork.isVisible()) {
      await fork.click();
      await fork.waitFor({ state: "hidden" });
    }
  });
  await test("Delayed language navigation cannot steal another project's active view", async () => {
    let release, intercepted;
    const pending = new Promise((resolve) => {
      release = resolve;
    });
    const seen = new Promise((resolve) => {
      intercepted = resolve;
    });
    const pattern = `**/ide/lsp/${lspId}/request`;
    const handler = async (route) => {
      if (route.request().postDataJSON().method !== "textDocument/definition")
        return route.continue();
      const response = await route.fetch();
      intercepted();
      await pending;
      await route.fulfill({ response });
    };
    await page.route(pattern, handler);
    try {
      const source = page
        .getByRole("textbox", { name: "Source editor main.ts", exact: true })
        .last();
      await source.click();
      await source.press("ControlOrMeta+Home");
      for (let i = 0; i < 17; i++) await source.press("ArrowRight");
      const received = page.waitForResponse(
        (response) =>
          response.url().includes(`/ide/lsp/${lspId}/request`) &&
          response.request().postDataJSON().method ===
            "textDocument/definition",
      );
      await editor()
        .getByRole("button", { name: "Definition", exact: true })
        .click();
      await seen;
      await selectProject(b);
      release();
      await (await received).finished();
      await page.evaluate(
        () =>
          new Promise((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(resolve)),
          ),
      );
      assert.equal(
        await page.locator(`#project-${b}`).getAttribute("aria-selected"),
        "true",
      );
      assert.equal(await page.locator(".files-editor:visible").count(), 0);
    } finally {
      release();
      await page.unroute(pattern, handler);
    }
  });
  await test("DAP Python breakpoint stepping stack and variables", async () => {
    await selectProject(b);
    await openFile("debug.py");
    const source = page
      .getByRole("textbox", { name: "Source editor debug.py", exact: true })
      .last();
    await source.click();
    await source.press("ControlOrMeta+Home");
    await source.press("ArrowDown");
    await editor()
      .getByRole("button", { name: "Toggle breakpoint", exact: true })
      .click();
    await view("Debug");
    await page
      .getByRole("button", { name: "Start debugging", exact: true })
      .click();
    await page
      .getByRole("button", {
        name: "Launch approved configuration",
        exact: true,
      })
      .click();
    await page.getByText("Stopped · breakpoint", { exact: true }).waitFor();
    await page
      .locator(".ide-variables")
      .getByText("40", { exact: true })
      .first()
      .waitFor();
    await shot("04-python-debugger-stopped.png");
    await page.getByRole("button", { name: "Step over", exact: true }).click();
    await eventually(
      async () => /42/.test(await page.locator(".ide-variables").innerText()),
      "stepped variable value",
    );
    await shot("05-debugger-variables.png");
    await page
      .getByRole("button", { name: "Stop debugger", exact: true })
      .click();
  });
  await test("Development terminal real shell input output resize and split", async () => {
    await view("Development terminal");
    await page
      .getByRole("button", { name: "New terminal", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Start approved shell", exact: true })
      .click();
    await page.locator(".xterm-helper-textarea").last().waitFor();
    await page.locator(".ws-terminal[data-connection=open]").last().waitFor();
    const input = page.locator(".xterm-helper-textarea").last();
    await input.focus();
    await input.pressSequentially("Write-Output '__ASF_P06_TERMINAL__'");
    await input.press("Enter");
    await eventually(
      async () =>
        /\r?\n__ASF_P06_TERMINAL__/.test(
          await page.locator(".xterm-screen").last().innerText(),
        ),
      "real shell output",
    );
    await shot("06-development-terminal.png");
    await page
      .getByRole("button", { name: "Split terminal view", exact: true })
      .last()
      .click();
    await eventually(
      () =>
        page
          .locator(".xterm-helper-textarea")
          .count()
          .then((n) => n >= 2),
      "split terminal views",
    );
    await page.setViewportSize({ width: 1280, height: 800 });
    await shot("07-terminal-split.png");
  });
  await test("Terminal reconnect and cross-project ownership", async () => {
    const terminals = (await get(b, "ide/state")).terminals;
    assert.ok(terminals.length);
    const denied = await mutate(
      a,
      `ide/terminals/${terminals[0].id}/stop`,
      {},
      true,
    );
    assert.equal(denied.status(), 404);
    const rejected = await page.evaluate(
      ({ base, a, id }) =>
        new Promise((resolve) => {
          const socket = new WebSocket(
            `${base.replace("http", "ws")}/api/projects/${a}/ide/terminals/${id}/socket`,
          );
          socket.onopen = () => {
            socket.close();
            resolve(false);
          };
          socket.onerror = () => resolve(true);
        }),
      { base, a, id: terminals[0].id },
    );
    assert.equal(rejected, true);
    await page.reload();
    await page.locator(".xterm-helper-textarea").last().waitFor();
    await shot("08-restored-ide.png");
  });
  await test("Multiple independent development terminal processes", async () => {
    const before = (await get(b, "ide/state")).terminals.filter(
      (terminal) => terminal.exit_code === null,
    );
    await view("Development terminal");
    await page
      .getByRole("button", { name: "New terminal", exact: true })
      .last()
      .click();
    await page
      .getByRole("button", { name: "Start approved shell", exact: true })
      .click();
    const after = await eventually(async () => {
      const terminals = (await get(b, "ide/state")).terminals.filter(
        (terminal) => terminal.exit_code === null,
      );
      return terminals.length > before.length ? terminals : false;
    }, "second terminal process");
    assert.ok(new Set(after.map((terminal) => terminal.pid)).size >= 2);
    await shot("08a-multiple-terminal-processes.png");
  });
  await test("EXT editor package installation activation and real formatter", async () => {
    await view("Extensions");
    if (
      !(await page
        .getByLabel("Extension package JSON", { exact: true })
        .isVisible())
    )
      await page
        .getByText("Install trusted local package", { exact: true })
        .click();
    const manifest = JSON.parse(
      await fs.readFile(
        path.join(root, "extensions/p06-editor/extension.json"),
        "utf8",
      ),
    );
    await page
      .getByLabel("Extension package JSON", { exact: true })
      .fill(JSON.stringify(manifest));
    await page
      .getByRole("button", { name: "Review package", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Trust and install", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Enable", exact: true })
      .last()
      .click();
    await openFile("debug.py");
    const source = page
      .getByRole("textbox", { name: "Source editor debug.py", exact: true })
      .last();
    await source.click();
    await source.press("ControlOrMeta+End");
    await source.pressSequentially("\n# trailing spaces   ");
    await page
      .getByText("Symbols, code actions and extension commands", {
        exact: true,
      })
      .click();
    await page
      .getByRole("button", { name: "Trim trailing whitespace", exact: true })
      .click();
    assert.ok(!(await source.innerText()).includes("spaces   "));
    await source.click();
    await source.press("ControlOrMeta+End");
    await source.pressSequentially("   ");
    await source.press("Alt+Shift+w");
    assert.ok(!(await source.innerText()).includes("spaces   "));
    await source.press("Shift+F10");
    const menu = page.getByRole("menu", { name: "Source editor actions" });
    await menu.waitFor();
    await menu
      .getByRole("menuitem", { name: "Trim source whitespace", exact: true })
      .click();
    await menu.waitFor({ state: "hidden" });
  });
  await test("EXT workspace contribution disable re-enable uninstall", async () => {
    await view("Extensions");
    const manifest = JSON.parse(
      await fs.readFile(
        path.join(root, "extensions/p06-workspace/extension.json"),
        "utf8",
      ),
    );
    if (
      !(await page
        .getByLabel("Extension package JSON", { exact: true })
        .isVisible())
    )
      await page
        .getByText("Install trusted local package", { exact: true })
        .click();
    await page
      .getByLabel("Extension package JSON", { exact: true })
      .fill(JSON.stringify(manifest));
    await page
      .getByRole("button", { name: "Review package", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Trust and install", exact: true })
      .click();
    const row = page
      .locator(".ide-extension-row")
      .filter({ hasText: "ASF Workspace Notes" });
    await row.getByRole("button", { name: "Enable", exact: true }).click();
    await row
      .getByRole("button", { name: "Workspace checklist", exact: true })
      .click();
    await page
      .getByText("Review current diagnostics before delivery.", {
        exact: false,
      })
      .waitFor();
    await shot("09-extension-workspace.png");
    await view("Extensions");
    await row.getByRole("button", { name: "Disable", exact: true }).click();
    await row.getByRole("button", { name: "Enable", exact: true }).click();
    await shot("10-extension-manager.png");
    await row.getByRole("button", { name: "Uninstall", exact: true }).click();
    await page
      .getByRole("dialog", { name: "Uninstall extension" })
      .getByRole("button", { name: "Uninstall extension", exact: true })
      .click();
    await row.waitFor({ state: "hidden" });
  });
  await test("EXT editor disable re-enable uninstall and restoration", async () => {
    await view("Extensions");
    const row = page
      .locator(".ide-extension-row")
      .filter({ hasText: "ASF Source Hygiene" });
    await row.getByRole("button", { name: "Disable", exact: true }).click();
    await row.getByRole("button", { name: "Enable", exact: true }).waitFor();
    await row.getByRole("button", { name: "Enable", exact: true }).click();
    await row.getByRole("button", { name: "Disable", exact: true }).waitFor();
    await row.getByRole("button", { name: "Uninstall", exact: true }).click();
    await page
      .getByRole("dialog", { name: "Uninstall extension" })
      .getByRole("button", { name: "Uninstall extension", exact: true })
      .click();
    await row.waitFor({ state: "hidden" });
    if (
      !(await page
        .getByLabel("Extension package JSON", { exact: true })
        .isVisible())
    )
      await page
        .getByText("Install trusted local package", { exact: true })
        .click();
    await page
      .getByLabel("Extension package JSON", { exact: true })
      .fill(
        await fs.readFile(
          path.join(root, "extensions/p06-editor/extension.json"),
          "utf8",
        ),
      );
    await page
      .getByRole("button", { name: "Review package", exact: true })
      .click();
    await page
      .getByRole("button", { name: "Trust and install", exact: true })
      .click();
    await row.getByRole("button", { name: "Enable", exact: true }).click();
  });
  await test("Light theme narrow desktop accessibility and no external requests", async () => {
    await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    const darkAudit = await page.evaluate(() =>
      axe.run(document, {
        runOnly: {
          type: "tag",
          values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
        },
      }),
    );
    await fs.writeFile(
      path.join(output, "browser-accessibility-dark.json"),
      JSON.stringify(darkAudit, null, 2),
    );
    assert.deepEqual(darkAudit.violations, []);
    await page.getByLabel("Theme", { exact: true }).selectOption("light");
    await shot("11-light-ide.png");
    await page.setViewportSize({ width: 1024, height: 768 });
    await shot("12-narrow-ide.png");
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth + 1,
      ),
      true,
    );
    await page.addScriptTag({ path: require.resolve("axe-core/axe.min.js") });
    const audit = await page.evaluate(() =>
      axe.run(document, {
        runOnly: {
          type: "tag",
          values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
        },
      }),
    );
    await fs.writeFile(
      path.join(output, "browser-accessibility.json"),
      JSON.stringify(audit, null, 2),
    );
    assert.deepEqual(
      audit.violations.map((v) => ({
        id: v.id,
        impact: v.impact,
        nodes: v.nodes.length,
      })),
      [],
    );
    assert.deepEqual(result.requests, []);
  });
  await test("Backend restart reconnect restores files and extension metadata without process auto-launch", async () => {
    await stop();
    await start();
    await page.reload();
    await page.getByRole("button", { name: "Views", exact: true }).waitFor();
    await mutate(b, "open");
    await page.reload();
    await selectProject(b);
    const state = await get(b, "ide/state");
    assert.equal(state.servers.length, 0);
    assert.equal(state.terminals.length, 0);
    assert.equal(state.debug_sessions.length, 0);
    assert.ok(state.extensions.some((e) => e.manifest.id === "asf.p06-editor"));
    await openFile("debug.py");
    await shot("13-restart-restored.png");
  });
  await test("V1 existing dashboard remains available", async () => {
    const legacy = await context.newPage();
    await legacy.goto(`${base}/?project=${a}`);
    await legacy.getByText("Agent Graph", { exact: true }).first().waitFor();
    await legacy.close();
  });
  assert.deepEqual(result.errors, []);
} catch (error) {
  console.error(error.stack);
  result.failure = error.message;
  if (a) result.ideFailureState = await get(a, "ide/state").catch(() => null);
  if (a && lspId)
    result.lspFailureEvents = await get(
      a,
      `ide/lsp/${lspId}/events?cursor=0`,
    ).catch(() => null);
  process.exitCode = 1;
} finally {
  result.finished = new Date().toISOString();
  if (page)
    result.measurements.push({
      browser: await page
        .evaluate(() => ({
          memory: performance.memory?.usedJSHeapSize,
          domNodes: document.querySelectorAll("*").length,
        }))
        .catch(() => null),
    });
  await context?.close();
  await browser?.close();
  await stop();
  await fs.writeFile(
    path.join(output, "browser-results.json"),
    JSON.stringify(result, null, 2),
  );
}
