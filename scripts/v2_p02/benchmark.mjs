import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs/promises";
import { gzipSync } from "node:zlib";
import os from "node:os";

const root = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium } = require("playwright");
const out = path.join(root, ".factory/v2/p02");
const base = process.env.ASF_P02_URL ?? "http://127.0.0.1:5182/v2.html";
const results = {
  schema: 1,
  measuredAt: new Date().toISOString(),
  environment: {
    platform: os.platform(),
    arch: os.arch(),
    cpu: os.cpus()[0].model,
    logicalCpus: os.cpus().length,
    ramBytes: os.totalmem(),
    node: process.version,
  },
  method:
    "Built preview; Edge headless; five independent browser-cache-cold contexts and warm reloads per fixture; no CPU/network throttling; timings include two animation frames. Process/OS filesystem cache is not flushed. Actions through public layout APIs or actual controls. No backend, LSP, DAP or agent execution.",
  assets: [],
  samples: [],
  summary: {},
};
for (const filename of await fs.readdir(path.join(out, "dist/assets"))) {
  const data = await fs.readFile(path.join(out, "dist/assets", filename));
  results.assets.push({
    filename,
    bytes: data.length,
    gzipBytes: gzipSync(data).length,
  });
}
const browser = await chromium.launch({ channel: "msedge", headless: true });
results.environment.browser = browser.version();
for (const scenario of ["conversation", "long"]) {
  for (let run = 0; run < 5; run++) {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 960 },
    });
    const page = await context.newPage();
    const client = await context.newCDPSession(page);
    await client.send("Network.clearBrowserCache");
    await client.send("Performance.enable");
    await page.goto(`${base}?scenario=${scenario}`);
    await page.waitForFunction(
      () => performance.getEntriesByName("asf-p02-shell-ready").length > 0,
    );
    await page.evaluate(() => document.fonts.ready);
    const sample = { scenario, run, cold: await metrics(page), actions: [] };
    await page.reload();
    await page.waitForFunction(
      () => performance.getEntriesByName("asf-p02-shell-ready").length > 0,
    );
    sample.warm = await metrics(page);
    for (const preset of ["Code", "Agents", "Review", "Focus"]) {
      const ms = await page.evaluate(async (preset) => {
        const start = performance.now();
        window.__ASF_P02__.controller.setPreset(preset);
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        );
        return performance.now() - start;
      }, preset);
      if (["Code", "Review"].includes(preset))
        await page.locator(".cm-editor").first().waitFor();
      sample.actions.push({
        action: `preset-${preset}`,
        ms,
        definition:
          "Public API to two frames; first lazy editor download measured separately",
      });
    }
    for (const theme of ["dark", "light"]) {
      const start = performance.now();
      await page
        .getByRole("combobox", { name: "Theme", exact: true })
        .selectOption(theme);
      await frames(page);
      sample.actions.push({
        action: `theme-${theme}`,
        ms: performance.now() - start,
        definition:
          "Automation command through native select to two frames; includes transport latency",
      });
    }
    await page.getByRole("tab", { name: "Code", exact: true }).click();
    await page.locator(".cm-editor").waitFor();
    const sash = await page
      .locator(".dv-sash.dv-enabled")
      .first()
      .boundingBox();
    const startResize = performance.now();
    await page.mouse.move(sash.x + 2, sash.y + 80);
    await page.mouse.down();
    await page.mouse.move(sash.x + 34, sash.y + 80, { steps: 4 });
    await page.mouse.up();
    await frames(page);
    sample.actions.push({
      action: "pointer-resize",
      ms: performance.now() - startResize,
      definition:
        "Four pointer steps plus automation roundtrips and two frames; not an INP benchmark",
    });
    const many = await page.evaluate(async () => {
      const model = window.__ASF_P02__.controller;
      const start = performance.now();
      for (const file of model.files.slice(0, 30)) model.openFile(file.path);
      await new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      );
      return {
        ms: performance.now() - start,
        panels: model.api.panels.length,
        treeNodes: document.querySelectorAll('[role="treeitem"]').length,
        messageNodes: document.querySelectorAll(".ds-message").length,
      };
    });
    sample.manyTabs = many;
    sample.actions.push({
      action: "move-active-tab",
      ms: await page.evaluate(async () => {
        const start = performance.now();
        window.__ASF_P02__.controller.moveActive("left");
        await new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        );
        return performance.now() - start;
      }),
    });
    const perf = await client.send("Performance.getMetrics");
    sample.memory = Object.fromEntries(
      perf.metrics
        .filter((m) =>
          [
            "JSHeapUsedSize",
            "JSHeapTotalSize",
            "Nodes",
            "Documents",
            "LayoutCount",
            "RecalcStyleCount",
          ].includes(m.name),
        )
        .map((m) => [m.name, m.value]),
    );
    sample.loadedResources = await page.evaluate(() =>
      performance
        .getEntriesByType("resource")
        .filter((e) => /\.(js|css|woff2)/.test(e.name))
        .map((e) => ({
          name: e.name.split("/").at(-1),
          durationMs: e.duration,
          transferSize: e.transferSize,
          decodedBodySize: e.decodedBodySize,
        })),
    );
    results.samples.push(sample);
    console.log(
      `${scenario} ${run + 1}/5 shell ${sample.cold.shellReadyMs.toFixed(1)} / ${sample.warm.shellReadyMs.toFixed(1)} ms`,
    );
    await context.close();
  }
}
function aggregate(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return {
    n: values.length,
    min: sorted[0],
    median: sorted[Math.floor(sorted.length / 2)],
    max: sorted.at(-1),
  };
}
for (const scenario of ["conversation", "long"]) {
  const samples = results.samples.filter((s) => s.scenario === scenario);
  results.summary[scenario] = {
    coldShellMs: aggregate(samples.map((s) => s.cold.shellReadyMs)),
    warmShellMs: aggregate(samples.map((s) => s.warm.shellReadyMs)),
    treeNodes: samples[0].cold.treeNodes,
    messages: samples[0].cold.messageNodes,
    manyTabsMs: aggregate(samples.map((s) => s.manyTabs.ms)),
    heapUsedBytes: aggregate(samples.map((s) => s.memory.JSHeapUsedSize)),
    actions: Object.fromEntries(
      samples[0].actions.map((a) => [
        a.action,
        aggregate(
          samples.map((s) => s.actions.find((t) => t.action === a.action).ms),
        ),
      ]),
    ),
  };
}
await browser.close();
await fs.writeFile(
  path.join(out, "performance.json"),
  JSON.stringify(results, null, 2),
);
console.log(JSON.stringify(results.summary));
async function frames(page) {
  await page.evaluate(
    () =>
      new Promise((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(resolve)),
      ),
  );
}
async function metrics(page) {
  return page.evaluate(() => ({
    shellReadyMs: performance.getEntriesByName("asf-p02-shell-ready")[0]
      .startTime,
    domContentLoadedMs:
      performance.getEntriesByType("navigation")[0].domContentLoadedEventEnd,
    fixtureFiles: window.__ASF_P02__.controller.files.length,
    treeNodes: document.querySelectorAll('[role="treeitem"]').length,
    messageNodes: document.querySelectorAll(".ds-message").length,
    documentNodes: document.querySelectorAll("*").length,
  }));
}
