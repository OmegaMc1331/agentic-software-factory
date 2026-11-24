import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs/promises";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const out = path.join(root, ".factory/v2/p02");
const shots = path.join(out, "screenshots");
await fs.mkdir(shots, { recursive: true });
const base = process.env.ASF_P02_URL ?? "http://127.0.0.1:5182/v2.html";
const browser = await chromium.launch({ channel: "msedge", headless: true });
const context = await browser.newContext({
  viewport: { width: 1440, height: 960 },
  colorScheme: "light",
});
const page = await context.newPage();
page.setDefaultTimeout(10000);
const results = {
  browser: browser.version(),
  base,
  startedAt: new Date().toISOString(),
  tests: [],
  audits: [],
  contrasts: [],
  geometry: [],
  screenshots: [],
  consoleErrors: [],
  pageErrors: [],
  unexpectedRequests: [],
  cspViolations: [],
};
let expectedFailure = false;
page.on("console", (message) => {
  if (message.type() === "error")
    results.consoleErrors.push({
      text: message.text(),
      expected: expectedFailure,
    });
});
page.on("pageerror", (error) =>
  results.pageErrors.push({
    message: error.message,
    expected: expectedFailure,
  }),
);
page.on("request", (request) => {
  const url = new URL(request.url());
  if (url.origin !== new URL(base).origin || url.pathname.startsWith("/api/"))
    results.unexpectedRequests.push(request.url());
});
await page.addInitScript(() => {
  window.__P02_CSP = [];
  document.addEventListener("securitypolicyviolation", (event) =>
    window.__P02_CSP.push({
      directive: event.violatedDirective,
      uri: event.blockedURI,
    }),
  );
});
const save = async () =>
  fs.writeFile(
    path.join(out, "browser-results.json"),
    JSON.stringify(results, null, 2),
  );
async function test(name, fn) {
  const start = performance.now();
  try {
    const evidence = await fn();
    results.tests.push({
      name,
      status: "PASS",
      ms: performance.now() - start,
      evidence,
    });
    console.log(`PASS ${name}`);
  } catch (error) {
    results.tests.push({
      name,
      status: "FAIL",
      ms: performance.now() - start,
      error: error.stack,
    });
    console.error(`FAIL ${name}: ${error.message.slice(0, 350)}`);
    await page
      .screenshot({
        path: path.join(out, "failure.png"),
        animations: "disabled",
      })
      .catch(() => undefined);
  }
  await save();
}
const settle = () =>
  page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  );
async function open(scenario = "conversation", extra = "") {
  await page.goto(`${base}?scenario=${scenario}${extra}`);
  if (!extra.includes("gallery"))
    await page.waitForFunction(
      () => window.__ASF_P02__?.controller.state.ready,
    );
  else
    await page
      .getByRole("heading", { name: "Interface foundations" })
      .waitFor();
  await page.evaluate(() => document.fonts.ready);
  await settle();
}
async function fresh(scenario = "conversation", extra = "") {
  await page.goto(base);
  await page.evaluate(() => localStorage.clear());
  await open(scenario, extra);
}
async function screenshot(name) {
  const filename = `${name}.png`;
  await page.screenshot({
    path: path.join(shots, filename),
    animations: "disabled",
  });
  results.screenshots.push(filename);
}
const theme = (value) =>
  page
    .getByRole("combobox", { name: "Theme", exact: true })
    .selectOption(value);
const preset = (value) =>
  page.getByRole("tab", { name: value, exact: true }).click();
const dockTab = (id) => page.locator(`.dv-tab[data-tab-panel-id="${id}"]`);
const apiSnapshot = () =>
  page.evaluate(() => {
    const api = window.__ASF_P02__.controller.api;
    return {
      groups: api.groups.map((g) => ({
        id: g.id,
        panels: g.panels.map((p) => p.id),
        width: g.width,
        height: g.height,
      })),
      active: api.activePanel?.id,
      panels: api.panels.map((p) => p.id),
    };
  });

await test("Theme: system changes, manual override, persistence, editor integration", async () => {
  await fresh("code");
  await page.locator(".cm-editor").waitFor();
  await theme("system");
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForFunction(
    () => document.documentElement.dataset.asfTheme === "dark",
  );
  assert.equal(
    await page
      .locator(".cm-editor")
      .first()
      .evaluate((e) => getComputedStyle(e).backgroundColor),
    "rgb(21, 24, 30)",
  );
  await theme("light");
  await page.emulateMedia({ colorScheme: "light" });
  await page.emulateMedia({ colorScheme: "dark" });
  assert.equal(
    await page.locator("html").getAttribute("data-asf-theme"),
    "light",
  );
  await page.reload();
  await page.locator(".cm-editor").waitFor();
  assert.equal(
    await page
      .getByRole("combobox", { name: "Theme", exact: true })
      .inputValue(),
    "light",
  );
  await page.emulateMedia({ colorScheme: "light" });
});

await test("Native dialog: keyboard trap, validation, Escape, focus return", async () => {
  await fresh("conversation");
  const trigger = page.getByRole("button", {
    name: "Workspace preferences",
    exact: true,
  });
  await trigger.click();
  const dialog = page.getByRole("dialog", {
    name: "Workspace preferences",
    exact: true,
  });
  await dialog.waitFor();
  await screenshot("preferences-dialog-light-1440");
  for (let i = 0; i < 14; i++) {
    await page.keyboard.press(i % 4 === 0 ? "Shift+Tab" : "Tab");
    assert.equal(
      await dialog.evaluate((e) => e.contains(document.activeElement)),
      true,
    );
  }
  await page.getByRole("switch", { name: "Compact interface density" }).click();
  assert.equal(
    await page.locator("html").getAttribute("data-density"),
    "compact",
  );
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "detached" });
  assert.equal(
    await trigger.evaluate((e) => e === document.activeElement),
    true,
  );
  await page.reload();
  await page.waitForFunction(() => window.__ASF_P02__?.controller.state.ready);
  assert.equal(
    await page.locator("html").getAttribute("data-density"),
    "compact",
  );
  await screenshot("focus-compact-light-1440");
});

await test("Workspace search: keyboard navigation, empty results, Escape", async () => {
  await fresh("conversation");
  await page.keyboard.press("Control+k");
  const input = page.getByRole("combobox", {
    name: "Search fixture files and commands",
  });
  await input.fill("session.ts");
  await page.keyboard.press("Enter");
  await page.locator('.dv-tab[aria-label="session.ts"]').waitFor();
  await page.locator(".cm-editor").waitFor();
  await page.keyboard.press("Control+k");
  await input.fill("file-does-not-exist");
  await page
    .getByRole("heading", { name: "No matching file or command" })
    .waitFor();
  await page.keyboard.press("Escape");
  assert.equal(await input.inputValue(), "");
  await page.keyboard.press("Escape");
  await page
    .getByRole("dialog", { name: "Search workspace" })
    .waitFor({ state: "detached" });
});

await test("Dockview: four presets, collapse/restore, group focus", async () => {
  await fresh("conversation");
  const snapshots = {};
  for (const name of ["Focus", "Code", "Agents", "Review"]) {
    await preset(name);
    await settle();
    snapshots[name] = await apiSnapshot();
    assert.ok(snapshots[name].groups.length >= 2);
  }
  await preset("Code");
  await page
    .getByRole("button", { name: "Toggle explorer", exact: true })
    .click();
  assert.equal(await dockTab("explorer").count(), 0);
  await page
    .getByRole("button", { name: "Toggle explorer", exact: true })
    .click();
  await dockTab("explorer").waitFor();
  const before = (await apiSnapshot()).active;
  await page.keyboard.press("F6");
  assert.notEqual((await apiSnapshot()).active, before);
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page
    .getByRole("menuitem", { name: "Show Agents", exact: true })
    .click();
  await dockTab("agents").waitFor();
  return snapshots;
});

await test("Dockview: actual sash pointer resize, native tab drag and keyboard reorder", async () => {
  await fresh("code");
  await page.locator(".cm-editor").waitFor();
  const before = await apiSnapshot();
  const sash = await page.locator(".dv-sash.dv-enabled").first().boundingBox();
  await page.mouse.move(
    sash.x + sash.width / 2,
    sash.y + Math.min(100, sash.height / 2),
  );
  await page.mouse.down();
  await page.mouse.move(sash.x + 55, sash.y + Math.min(100, sash.height / 2), {
    steps: 8,
  });
  await page.mouse.up();
  await settle();
  const after = await apiSnapshot();
  assert.ok(
    Math.abs(
      after.groups.find((g) => g.panels.includes("explorer")).width -
        before.groups.find((g) => g.panels.includes("explorer")).width,
    ) > 30,
  );
  await dockTab("explorer").click();
  await page.getByRole("button", { name: "Views", exact: true }).click();
  await page
    .getByRole("menuitem", { name: "Resize active panel…", exact: true })
    .click();
  await page
    .getByRole("spinbutton", { name: "Panel width", exact: true })
    .fill("340");
  await page
    .getByRole("button", { name: "Apply dimensions", exact: true })
    .click();
  await settle();
  assert.ok(
    Math.abs(
      (await apiSnapshot()).groups.find((g) => g.panels.includes("explorer"))
        .width - 340,
    ) < 2,
  );
  await dockTab("conversation").dragTo(dockTab("editor"), {
    targetPosition: { x: 70, y: 15 },
  });
  await settle();
  const dragged = await apiSnapshot();
  assert.ok(
    dragged.groups.some(
      (g) => g.panels.includes("conversation") && g.panels.includes("editor"),
    ),
    "Native tab drag must change the actual Dockview group",
  );
  await dockTab("conversation").click();
  const orderBefore = (await apiSnapshot()).groups.find((g) =>
    g.panels.includes("conversation"),
  ).panels;
  const direction =
    orderBefore.indexOf("conversation") === 0 ? "right" : "left";
  await page
    .getByRole("button", { name: `Move active tab ${direction}`, exact: true })
    .click();
  const reordered = (await apiSnapshot()).groups.find((g) =>
    g.panels.includes("conversation"),
  ).panels;
  assert.notDeepEqual(reordered, orderBefore);
  return { before, after, dragged, reordered };
});

await test("CodeMirror: independent files, synchronized split, buffers and layout survive reload", async () => {
  await fresh("code");
  await page.locator(".cm-content[contenteditable=true]").waitFor();
  for (const appearance of ["light", "dark"]) {
    await theme(appearance);
    await settle();
    await screenshot(`ux05-code-${appearance}-1440`);
  }
  await theme("light");
  await page.locator(".cm-content[contenteditable=true]").click();
  await page.keyboard.press("Control+Home");
  await page.keyboard.insertText("// P02 local buffer: café λ\n");
  await page
    .getByRole("button", { name: "Split active editor", exact: true })
    .click();
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".cm-content[contenteditable=true]").length ===
      2,
  );
  for (const editor of await page
    .locator(".cm-content[contenteditable=true]")
    .all())
    assert.ok(
      (await editor.innerText()).startsWith("// P02 local buffer: café λ"),
    );
  await page.locator(".cm-content[contenteditable=true]").last().click();
  await page.keyboard.press("Control+End");
  await page.keyboard.insertText("\n// synchronized peer\n");
  assert.ok(
    (
      await page
        .locator(".cm-content[contenteditable=true]")
        .first()
        .innerText()
    ).includes("synchronized peer"),
  );
  const snapshot = await page.evaluate(() =>
    window.__ASF_P02__.controller.api.toJSON(),
  );
  await page
    .getByRole("button", { name: "Save preview", exact: true })
    .first()
    .click();
  await page.evaluate(() => window.__ASF_P02__.controller.persist());
  await page.reload();
  await page.waitForFunction(
    () =>
      document.querySelectorAll(".cm-content[contenteditable=true]").length ===
      2,
  );
  const restored = await page.evaluate(() =>
    window.__ASF_P02__.controller.api.toJSON(),
  );
  assert.deepEqual(restored.grid, snapshot.grid);
  assert.deepEqual(restored.panels, snapshot.panels);
  assert.ok(
    (
      await page
        .locator(".cm-content[contenteditable=true]")
        .first()
        .innerText()
    ).includes("synchronized peer"),
  );
  await page.keyboard.press("Control+k");
  await page
    .getByRole("combobox", { name: "Search fixture files and commands" })
    .fill("session.ts");
  await page.keyboard.press("Enter");
  await page.locator('.dv-tab[aria-label="session.ts"]').waitFor();
  assert.equal(
    await page.evaluate(() =>
      window.__ASF_P02__.controller
        .buffer("src/ui/session.ts")
        .text.startsWith("// P02"),
    ),
    false,
  );
});

await test("Conversation: fixture-only response, drafts and project isolation", async () => {
  await fresh("empty");
  const composer = page.getByRole("textbox", {
    name: "Message to orchestrator",
  });
  await composer.fill("Check the checkpoint owner");
  await page.keyboard.press("Control+Enter");
  await page
    .getByText(
      "Your request is recorded in this local preview. No agent or runtime is connected.",
      { exact: true },
    )
    .waitFor();
  await composer.fill("Draft retained across reload");
  await page.evaluate(() => window.__ASF_P02__.controller.persist());
  await page.reload();
  await composer.waitFor();
  assert.equal(await composer.inputValue(), "Draft retained across reload");
  await page.getByRole("tab", { name: "Fieldnotes", exact: true }).click();
  await page.waitForURL(/project=fieldnotes/);
  await composer.waitFor();
  assert.equal(await composer.inputValue(), "");
  await page.getByRole("tab", { name: "Atlas", exact: true }).click();
  await page.waitForURL(/project=atlas/);
  await composer.waitFor();
  assert.equal(await composer.inputValue(), "Draft retained across reload");
});

await test("Mission attention: explicit checkpoint decision, permission-limited fixture reassignment", async () => {
  await fresh("attention");
  await page
    .getByRole("button", { name: "Choose an agent", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Reassign task", exact: true })
    .click();
  const dialog = page.getByRole("dialog", { name: "Reassign verification" });
  await dialog.waitFor();
  assert.equal(
    await page.locator('option[value="gemini"]').evaluate((e) => e.disabled),
    true,
  );
  await page
    .getByRole("button", { name: "Assign in fixture", exact: true })
    .click();
  await dialog.waitFor({ state: "detached" });
  const mission = await page.evaluate(
    () => window.__ASF_P02__.controller.state.mission,
  );
  assert.equal(mission.status, "running");
  assert.equal(mission.checkpoint.id, "cp-014");
});

await test("Review: real CodeMirror diff, read-only, navigation, fixture acceptance, publication disabled", async () => {
  await fresh("review");
  await page.locator(".cm-mergeView").waitFor();
  for (const appearance of ["light", "dark"]) {
    await theme(appearance);
    await settle();
    await screenshot(`ux06-review-${appearance}-1440`);
  }
  assert.ok((await page.locator(".cm-changedLine").count()) > 0);
  assert.equal(
    await page.locator(".cm-content[contenteditable=true]").count(),
    0,
  );
  await page.getByRole("button", { name: "Next change", exact: true }).click();
  await page
    .getByRole("button", { name: "Accept fixture changes", exact: true })
    .click();
  assert.equal(
    await page
      .getByRole("button", { name: "Accepted", exact: true })
      .isDisabled(),
    true,
  );
  assert.equal(
    await page
      .getByRole("button", { name: "Publish", exact: true })
      .isDisabled(),
    true,
  );
});

await test("Notifications: individually dismissible, empty state", async () => {
  await fresh("conversation");
  await page.getByRole("button", { name: /^Notifications \(/ }).click();
  const dismiss = page.getByRole("button", { name: /^Dismiss / });
  while (await dismiss.count()) await dismiss.first().click();
  await page.getByRole("heading", { name: "No notifications" }).waitFor();
  await page.keyboard.press("Escape");
});

await test("Recovery: corrupt persisted layout produces an explanation and a usable preset", async () => {
  await fresh("code");
  await page.addInitScript(() => {
    if (new URLSearchParams(location.search).has("corrupt"))
      localStorage.setItem(
        "asf.v2.p02.workspace.atlas.code",
        '{"version":999}',
      );
  });
  await open("code", "&corrupt=true");
  await page.getByText("Preview recovery", { exact: true }).waitFor();
  await page
    .getByRole("button", { name: "Restore preset", exact: true })
    .click();
  await page.locator(".cm-editor").waitFor();
  assert.equal(
    await page.getByText("Preview recovery", { exact: true }).count(),
    0,
  );
});

await test("Gallery: disabled controls, menus, context keyboard, tooltip, popover, validation and error boundary", async () => {
  await fresh("conversation", "&gallery=components");
  assert.equal(
    await page
      .getByRole("button", { name: "Unavailable", exact: true })
      .isDisabled(),
    true,
  );
  await page.getByRole("button", { name: "Gallery menu", exact: true }).click();
  assert.equal(
    await page
      .getByRole("menuitem", { name: "Add fixture reference", exact: true })
      .evaluate((e) => e === document.activeElement),
    true,
  );
  await page.keyboard.press("ArrowDown");
  assert.equal(
    await page
      .getByRole("menuitem", { name: "Open fixture review", exact: true })
      .evaluate((e) => e === document.activeElement),
    true,
  );
  await page.keyboard.press("ArrowDown");
  assert.equal(
    await page
      .getByRole("menuitem", { name: "Add fixture reference", exact: true })
      .evaluate((e) => e === document.activeElement),
    true,
  );
  await page.keyboard.press("Escape");
  const source = page.getByRole("button", {
    name: "Right-click or Shift F10",
    exact: true,
  });
  await source.focus();
  await page.keyboard.press("Shift+F10");
  const contextMenu = page.getByRole("menu", {
    name: "Gallery context actions",
  });
  await contextMenu.waitFor();
  const bounds = await contextMenu.boundingBox();
  assert.ok(
    bounds.x >= 0 &&
      bounds.y >= 0 &&
      bounds.x + bounds.width <= 1440 &&
      bounds.y + bounds.height <= 960,
  );
  await screenshot("context-menu-light-1440");
  await page.keyboard.press("Escape");
  assert.equal(
    await source.evaluate((e) => e === document.activeElement),
    true,
  );
  const info = page.getByRole("button", {
    name: "Control information",
    exact: true,
  });
  await info.focus();
  await page.getByRole("tooltip").waitFor();
  await page.keyboard.press("Escape");
  assert.equal(await page.getByRole("tooltip").count(), 0);
  await page
    .getByRole("button", { name: "Context details", exact: true })
    .click();
  await page.getByRole("dialog", { name: "Context budget" }).waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Open dialog", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Edit preview preferences" });
  await dialog.waitFor();
  await screenshot("gallery-dialog-light-1440");
  await page
    .getByRole("textbox", { name: "Dialog session name", exact: true })
    .fill("");
  assert.equal(
    await page.getByRole("button", { name: "Apply", exact: true }).isDisabled(),
    true,
  );
  await screenshot("gallery-validation-light-1440");
  await page.keyboard.press("Escape");
  const divider = page.getByRole("separator", { name: "Resize gallery panel" });
  await divider.focus();
  await page.keyboard.press("ArrowRight");
  assert.equal(await divider.getAttribute("aria-valuenow"), "218");
  await page.keyboard.press("End");
  assert.equal(await divider.getAttribute("aria-valuenow"), "420");
  expectedFailure = true;
  await page
    .getByRole("button", { name: "Show failed view", exact: true })
    .click();
  await screenshot("gallery-error-boundary-light-1440");
  await page.getByRole("button", { name: "Reload view", exact: true }).click();
  await page
    .getByText("The fixture view is available.", { exact: true })
    .waitFor();
  expectedFailure = false;
});

await test("Long content: bounded tree and messages, keyboard traversal, many tabs, terminal overflow", async () => {
  await fresh("long");
  const tree = page.getByRole("tree", { name: "Fixture project files" });
  await tree.waitFor();
  const initialRows = await page.getByRole("treeitem").count();
  assert.ok(initialRows > 8 && initialRows < 70);
  await tree.focus();
  await page.keyboard.press("End");
  await settle();
  assert.ok((await tree.getAttribute("aria-activedescendant"))?.length);
  assert.ok((await page.locator(".ds-message").count()) <= 24);
  const expand = page.getByRole("button", {
    name: "Read full message",
    exact: true,
  });
  if (await expand.count()) {
    await expand.click();
    assert.ok(
      (await page.locator(".ds-message-body").last().innerText()).length > 1400,
    );
    await page.getByRole("button", { name: "Show less", exact: true }).click();
  }
  await page.getByRole("button", { name: /Earlier \d+ messages/ }).click();
  assert.ok((await page.locator(".ds-message").count()) <= 24);
  await page.evaluate(() => {
    const model = window.__ASF_P02__.controller;
    model.setPreset("Code");
    for (const file of model.files
      .filter((f) => f.path.startsWith("src/generated/"))
      .slice(0, 30))
      model.openFile(file.path);
    model.showView("output");
  });
  assert.ok((await apiSnapshot()).panels.length >= 30);
  const output = page.locator(".v2-output-content");
  assert.equal(
    await output.evaluate((e) => e.scrollHeight > e.clientHeight),
    true,
  );
  await page.getByRole("button", { name: "Wrap lines", exact: true }).click();
  assert.equal(
    await page
      .getByRole("button", { name: "Wrap lines", exact: true })
      .getAttribute("aria-pressed"),
    "true",
  );
  await screenshot("long-code-light-1440");
  return {
    fixtureFiles: await page.evaluate(
      () => window.__ASF_P02__.controller.files.length,
    ),
    initialRows,
    panels: (await apiSnapshot()).panels.length,
  };
});

await test("Accessibility tree: named regions, tree, tabs, diagnostics and modal controls", async () => {
  await fresh("code");
  await page.locator(".cm-editor").waitFor();
  const client = await context.newCDPSession(page);
  const tree = await client.send("Accessibility.getFullAXTree");
  const nodes = tree.nodes
    .filter((node) => !node.ignored)
    .map((node) => ({ role: node.role?.value, name: node.name?.value }));
  assert.ok(
    nodes.some((n) => n.role === "region" && n.name === "Workspace panels"),
  );
  assert.ok(
    nodes.some((n) => n.role === "tree" && n.name === "Fixture project files"),
  );
  assert.ok(
    nodes.some((n) => n.role === "image" && n.name === "0 errors, 0 warnings"),
  );
  await page
    .getByRole("button", { name: "Workspace preferences", exact: true })
    .click();
  const dialogTree = await client.send("Accessibility.getFullAXTree");
  assert.ok(
    dialogTree.nodes.some(
      (n) =>
        n.role?.value === "dialog" &&
        n.name?.value === "Workspace preferences" &&
        !n.ignored,
    ),
  );
  await page.keyboard.press("Escape");
  await client.detach();
  return {
    exposed: nodes.filter((n) =>
      ["region", "tree", "tablist", "image"].includes(n.role),
    ),
  };
});

await test("Responsive visual matrix: four presets, two themes, 1920/1440/1280/1024", async () => {
  await fresh("conversation");
  for (const width of [1920, 1440, 1280, 1024]) {
    await page.setViewportSize({ width, height: 960 });
    for (const appearance of ["light", "dark"]) {
      await theme(appearance);
      for (const name of ["Focus", "Code", "Agents", "Review"]) {
        await preset(name);
        await settle();
        if (name === "Code") await page.locator(".cm-editor").waitFor();
        if (name === "Review") await page.locator(".cm-mergeView").waitFor();
        const geometry = await page.evaluate(() => ({
          pageWidth: document.documentElement.clientWidth,
          scrollWidth: document.documentElement.scrollWidth,
          panelRects: [...document.querySelectorAll(".dv-groupview")]
            .filter((e) => e.getBoundingClientRect().width)
            .map((e) => ({
              w: e.getBoundingClientRect().width,
              h: e.getBoundingClientRect().height,
            })),
          cssTheme: document.documentElement.dataset.asfTheme,
        }));
        results.geometry.push({ width, appearance, preset: name, ...geometry });
        assert.ok(
          geometry.scrollWidth <= width + 1,
          `${width} ${appearance} ${name}: page overflow`,
        );
        assert.ok(
          geometry.panelRects.every((r) => r.w >= 120 && r.h >= 80),
          "Usable pane dimensions",
        );
        if (width === 1024 && ["Code", "Review"].includes(name)) {
          const snapshot = await apiSnapshot();
          assert.ok(
            snapshot.groups.find((g) => g.panels.includes("explorer")).width <=
              300,
            "Narrow presets must prioritize working content over Explorer",
          );
        }
        if (
          width === 1440 ||
          (width === 1024 && ["Code", "Focus"].includes(name))
        )
          await screenshot(`${name.toLowerCase()}-${appearance}-${width}`);
      }
    }
  }
});

await test("Scenario screenshots: empty, running, attention and long conversation in both themes", async () => {
  await page.setViewportSize({ width: 1440, height: 960 });
  for (const scenario of ["empty", "running", "attention", "long"]) {
    await fresh(scenario);
    for (const appearance of ["light", "dark"]) {
      await theme(appearance);
      await settle();
      await screenshot(`${scenario}-${appearance}-1440`);
    }
  }
});

await test("Reduced motion, CSS/DPR zoom simulation and 150% text scaling", async () => {
  await fresh("code");
  await page.locator(".cm-editor").waitFor();
  await page.emulateMedia({ reducedMotion: "reduce" });
  const motion = await page.evaluate(() => ({
    panel: getComputedStyle(document.documentElement)
      .getPropertyValue("--motion-panel")
      .trim(),
    button: getComputedStyle(document.querySelector(".ds-button"))
      .transitionDuration,
  }));
  assert.equal(motion.panel, "0ms");
  assert.ok(
    motion.button.split(",").every((v) => Number.parseFloat(v) <= 0.01),
  );
  await page.setViewportSize({ width: 1024, height: 900 });
  await page.evaluate(() => (document.documentElement.style.fontSize = "24px"));
  await settle();
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
    true,
  );
  await screenshot("code-text-150-light-1024");
  await page.evaluate(() => (document.documentElement.style.fontSize = ""));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const zoomContext = await browser.newContext({
    viewport: { width: 720, height: 480 },
    deviceScaleFactor: 2,
  });
  const zoomPage = await zoomContext.newPage();
  await zoomPage.goto(`${base}?scenario=conversation`);
  await zoomPage.waitForFunction(
    () => window.__ASF_P02__?.controller.state.ready,
  );
  assert.equal(
    await zoomPage.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
    true,
  );
  await zoomPage.screenshot({
    path: path.join(shots, "focus-200-equivalent.png"),
    animations: "disabled",
  });
  results.screenshots.push("focus-200-equivalent.png");
  await zoomContext.close();
  return {
    motion,
    zoom: "720 CSS px, DPR 2: reflow simulation, not native browser zoom or OS scaling",
  };
});

await test("Accessibility: axe WCAG 2 A/AA, 2.1/2.2 AA and measured semantic contrast", async () => {
  await page.setViewportSize({ width: 1440, height: 960 });
  const axe = await fs.readFile(require.resolve("axe-core/axe.min.js"), "utf8");
  await page.route("**/__p02-test/axe.js", (route) =>
    route.fulfill({ body: axe, contentType: "application/javascript" }),
  );
  const failures = [];
  for (const scenario of [
    "conversation",
    "code",
    "review",
    "attention",
    "long",
    "gallery",
  ]) {
    await fresh(
      scenario === "gallery" ? "conversation" : scenario,
      scenario === "gallery" ? "&gallery=components" : "",
    );
    if (["code", "review"].includes(scenario))
      await page.locator(".cm-editor").first().waitFor();
    await page.addScriptTag({ url: "/__p02-test/axe.js" });
    for (const appearance of ["light", "dark"]) {
      await page
        .getByRole("combobox", {
          name: scenario === "gallery" ? "Gallery theme" : "Theme",
          exact: true,
        })
        .selectOption(appearance);
      await settle();
      const audit = await page.evaluate(async () => {
        const result = await window.axe.run(document, {
          runOnly: {
            type: "tag",
            values: ["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"],
          },
        });
        return {
          violations: result.violations,
          incomplete: result.incomplete,
          passes: result.passes.length,
        };
      });
      results.audits.push({ scenario, appearance, ...audit });
      if (audit.violations.length)
        failures.push({
          scenario,
          appearance,
          ids: audit.violations.map((v) => ({
            id: v.id,
            nodes: v.nodes.map((n) => n.target),
          })),
        });
      if (scenario === "gallery") {
        await page.locator(".v2-gallery-scroll").evaluate((e) => {
          e.scrollTop = 0;
        });
        await screenshot(`gallery-${appearance}-1440`);
        for (const [title, slug] of [
          ["Feedback and recovery", "feedback"],
          ["Code and tools", "code"],
        ]) {
          await page
            .getByRole("heading", { name: title, exact: true })
            .scrollIntoViewIfNeeded();
          await screenshot(`gallery-${slug}-${appearance}-1440`);
        }
      }
    }
  }
  for (const appearance of ["light", "dark"]) {
    await page
      .getByRole("combobox", { name: "Gallery theme", exact: true })
      .selectOption(appearance);
    const pairs = await page.evaluate(() => {
      const probe = document.createElement("span");
      document.body.append(probe);
      const color = (token) => {
        probe.style.color = `var(${token})`;
        return getComputedStyle(probe).color;
      };
      const pairs = [
        ["--color-text", "--color-panel", 4.5],
        ["--color-text-secondary", "--color-panel", 4.5],
        ["--color-text-muted", "--color-panel-muted", 4.5],
        ["--color-selection-text", "--color-selection", 4.5],
        ["--color-on-accent", "--color-accent", 4.5],
        ["--color-success", "--color-success-bg", 4.5],
        ["--color-warning", "--color-warning-bg", 4.5],
        ["--color-danger", "--color-danger-bg", 4.5],
        ["--code-comment", "--code-bg", 4.5],
        ["--code-type", "--code-bg", 4.5],
        ["--code-keyword", "--code-bg", 4.5],
        ["--code-number", "--code-bg", 4.5],
        ["--code-string", "--code-bg", 4.5],
        ["--diff-added-text", "--diff-added-bg", 4.5],
        ["--diff-removed-text", "--diff-removed-bg", 4.5],
        ["--color-text", "--diff-added-bg", 4.5],
        ["--color-text", "--diff-removed-bg", 4.5],
        ["--terminal-text", "--terminal-bg", 4.5],
        ["--color-focus", "--color-panel", 3],
        ["--color-control-border", "--color-panel", 3],
      ].map(([fg, bg, min]) => ({
        fg,
        bg,
        min,
        foreground: color(fg),
        background: color(bg),
      }));
      probe.remove();
      return pairs;
    });
    const luminance = (rgb) =>
      rgb
        .match(/[\d.]+/g)
        .slice(0, 3)
        .map(Number)
        .map((x) => x / 255)
        .map((x) => (x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4))
        .reduce((sum, x, i) => sum + x * [0.2126, 0.7152, 0.0722][i], 0);
    for (const pair of pairs) {
      const a = luminance(pair.foreground);
      const b = luminance(pair.background);
      const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
      results.contrasts.push({ appearance, ...pair, ratio });
      if (ratio < pair.min)
        failures.push({ appearance, contrast: pair, ratio });
    }
  }
  assert.deepEqual(failures, []);
});

await test("Network/CSP: no backend operations, external resources or unexpected runtime errors", async () => {
  results.cspViolations = await page.evaluate(() => window.__P02_CSP);
  assert.deepEqual(results.unexpectedRequests, []);
  assert.deepEqual(results.cspViolations, []);
  assert.deepEqual(
    results.pageErrors.filter((error) => !error.expected),
    [],
  );
  assert.deepEqual(
    results.consoleErrors.filter((error) => !error.expected),
    [],
  );
});
results.completedAt = new Date().toISOString();
results.summary = {
  passed: results.tests.filter((t) => t.status === "PASS").length,
  failed: results.tests.filter((t) => t.status === "FAIL").length,
  screenshots: results.screenshots.length,
  axeRuns: results.audits.length,
};
await save();
await browser.close();
console.log(JSON.stringify(results.summary));
process.exitCode = results.summary.failed ? 1 : 0;
