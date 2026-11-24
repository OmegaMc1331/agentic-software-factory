import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import { performance } from 'node:perf_hooks';
import { execFileSync } from 'node:child_process';
import { gzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const state = path.join(root, '.factory/v2/p01');
const config = JSON.parse(fs.readFileSync(path.join(state, 'config.json'), 'utf8'));
const base = process.env.P01_BASE_URL ?? 'http://127.0.0.1:4387';
const rows = [];
const output = path.join(state, 'benchmarks.json');
const frames = async page => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));

function reset(project, candidate) {
  const folder = config.projects[project];
  if (fs.readFileSync(path.join(folder, '.asf-p01-fixture'), 'utf8') !== 'ASF_P01_OWNED_FIXTURE_V1') throw new Error('unowned fixture');
  for (const prefix of ['workspace', 'extension']) {
    const filename = path.join(folder, `.factory/p01/${prefix}-${candidate}.json`);
    if (fs.existsSync(filename)) fs.unlinkSync(filename);
  }
}

function memory(ids) {
  if (process.platform !== 'win32') return { status: 'not measured on this platform' };
  const numeric = ids.filter(id => Number.isInteger(id) && id > 0).join(',');
  if (!numeric) return [];
  const command = `Get-Process -Id ${numeric} -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`;
  return JSON.parse(execFileSync('powershell.exe', ['-NoProfile', '-Command', command], { encoding: 'utf8' }) || '[]');
}

async function api(page, suffix) {
  return page.evaluate(async suffix => (await fetch('/api' + suffix)).json(), suffix);
}

for (const candidate of ['monaco', 'codemirror']) {
  for (const project of ['small', 'medium', 'large']) {
    for (let repetition = 1; repetition <= 3; repetition++) {
      reset(project, candidate);
      const begin = performance.now();
      const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-precise-memory-info'] });
      const launchMs = performance.now() - begin;
      const context = await browser.newContext({ viewport: { width: 1500, height: 960 } });
      const page = await context.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      const navigation = performance.now();
      await page.goto(`${base}/${candidate}/?project=${project}`);
      await page.waitForFunction(() => window.p01?.workbench.ready, null, { timeout: 60000 });
      await frames(page);
      const newBrowserProjectReadyMs = performance.now() - navigation;
      const entries = await page.evaluate(() => window.p01.workbench.tree.length);
      const measure = async operation => { const start = performance.now(); await operation(); await frames(page); return performance.now() - start; };
      const openFileMs = await measure(() => page.evaluate(() => window.p01.workbench.open('src/main.ts')));
      await page.evaluate(() => window.p01.workbench.open('src/lib.ts'));
      const tabSwitchMs = await measure(() => page.evaluate(() => window.p01.workbench.open('src/main.ts')));
      await page.evaluate(() => window.p01.workbench.dock.getPanel('file:src/lib.ts').api.close());
      const splitMs = await measure(() => page.evaluate(() => window.p01.workbench.open('src/lib.ts', true)));
      const largeFileMs = await measure(() => page.evaluate(() => window.p01.workbench.open('src/large.ts')));
      await page.evaluate(() => window.p01.workbench.handles.get('src/large.ts').focus());
      const inputToTwoFramesMs = await measure(() => page.keyboard.type('x'));
      const heap = await page.evaluate(() => ({ used: performance.memory?.usedJSHeapSize, total: performance.memory?.totalJSHeapSize }));
      const resources = await page.evaluate(() => performance.getEntriesByType('resource').map(item => ({ name: new URL(item.name).pathname, transferSize: item.transferSize, encodedBodySize: item.encodedBodySize, duration: item.duration })));
      const browserCdp = await browser.newBrowserCDPSession();
      const processInfo = await browserCdp.send('SystemInfo.getProcessInfo');
      const owned = await api(page, '/processes');
      const processMemory = memory([...processInfo.processInfo.map(item => item.id), owned.backend_pid, ...owned.protocol.filter(item => item.running).map(item => item.pid)]);
      await page.evaluate(() => window.p01.workbench.persist());
      const warmReloadMs = await measure(async () => { await page.reload(); await page.waitForFunction(() => window.p01?.workbench.ready); await page.waitForFunction(() => window.p01.workbench.handles.has('src/large.ts')); });
      rows.push({ candidate, project, repetition, launchMs, newBrowserProjectReadyMs, coldProcessTotalMs: launchMs + newBrowserProjectReadyMs, entries, openFileMs, tabSwitchMs, splitMs, largeFileMs, inputToTwoFramesMs, warmReloadMs, heap, processMemory, browserProcesses: processInfo.processInfo, resources, errors });
      fs.writeFileSync(output, JSON.stringify({ schemaVersion: 1, measuredAt: new Date().toISOString(), method: 'New headless Edge process per sample; OS disk caches not flushed. Warm reload retains HTTP/browser caches and persisted layout. Input timing is keyboard roundtrip plus two animation frames, not INP.', rows }, null, 2));
      console.log(`${candidate} ${project} ${repetition}: ready=${newBrowserProjectReadyMs.toFixed(1)}ms open=${openFileMs.toFixed(1)}ms warm=${warmReloadMs.toFixed(1)}ms`);
      await browser.close();
    }
  }
}

const bundle = {};
function assets(folder) { return fs.readdirSync(folder, { withFileTypes: true }).flatMap(item => item.isDirectory() ? assets(path.join(folder, item.name)) : [path.join(folder, item.name)]); }
for (const candidate of ['monaco', 'codemirror']) {
  const folder = path.join(state, 'dist', candidate);
  const files = assets(folder).map(filename => { const data = fs.readFileSync(filename); return { path: path.relative(folder, filename).replaceAll('\\', '/'), bytes: data.length, gzipBytes: gzipSync(data).length }; });
  bundle[candidate] = { files, totalBytes: files.reduce((sum, item) => sum + item.bytes, 0), gzipBytes: files.reduce((sum, item) => sum + item.gzipBytes, 0) };
}
fs.writeFileSync(path.join(state, 'bundle-sizes.json'), JSON.stringify(bundle, null, 2));
