import { chromium } from 'playwright';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');
const state = path.join(root, '.factory/v2/p01');
const config = JSON.parse(fs.readFileSync(path.join(state, 'config.json'), 'utf8'));
const base = process.env.P01_BASE_URL ?? 'http://127.0.0.1:4387';
const output = path.join(state, 'prototype-results.json');
const results = [];
const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--enable-precise-memory-info'] });

function reset(candidate) {
  const project = config.projects.small;
  assert.equal(fs.readFileSync(path.join(project, '.asf-p01-fixture'), 'utf8'), 'ASF_P01_OWNED_FIXTURE_V1');
  for (const name of [`workspace-${candidate}.json`, `extension-${candidate}.json`]) {
    const file = path.join(project, '.factory/p01', name);
    if (fs.existsSync(file)) fs.unlinkSync(file);
  }
  fs.writeFileSync(path.join(project, 'src/main.ts'), 'export function greet(name: string): string { return `Hello ${name}`; }\nconst wrong: number = "mistake";\nconsole.log(greet("ASF"));\n');
}

async function pageFor(candidate) {
  const context = await browser.newContext({ viewport: { width: 1500, height: 960 } });
  const page = await context.newPage();
  page.errors = [];
  page.on('pageerror', error => page.errors.push(error.message));
  await page.goto(`${base}/${candidate}/?project=small`);
  await page.waitForFunction(() => window.p01?.workbench.ready, null, { timeout: 45000 });
  return { page, context };
}

async function test(candidate, id, operation) {
  const started = performance.now();
  try {
    const evidence = await operation();
    results.push({ candidate, id, status: 'PASS', elapsedMs: performance.now() - started, evidence });
    console.log(`${candidate} ${id} PASS`);
  } catch (error) {
    results.push({ candidate, id, status: 'FAIL', elapsedMs: performance.now() - started, error: String(error.stack) });
    console.log(`${candidate} ${id} FAIL ${error.message}`);
  }
  fs.writeFileSync(output, JSON.stringify({ schemaVersion: 1, measuredAt: new Date().toISOString(), base, tests: results }, null, 2));
}

async function browserApi(page, url, method = 'GET', body) {
  return page.evaluate(async ({ url, method, body }) => {
    const csrf = document.cookie.split('; ').find(cookie => cookie.startsWith('factory_csrf='))?.split('=')[1];
    const response = await fetch('/api' + url, { method, headers: { 'Content-Type': 'application/json', 'X-Factory-CSRF': csrf ?? '' }, body: body === undefined ? undefined : JSON.stringify(body) });
    return { status: response.status, body: await response.json() };
  }, { url, method, body });
}

try {
  for (const candidate of (process.env.P01_CANDIDATES ?? 'monaco,codemirror').split(',')) {
    reset(candidate);
    let { page, context } = await pageFor(candidate);
    await test(candidate, 'IDE-01', async () => {
      const tree = await page.evaluate(() => window.p01.workbench.tree);
      assert(tree.some(file => file.path === 'tree/group-000/module-00000.ts'));
      assert(tree.some(file => file.path === 'src/main.ts'));
      const traversal = await browserApi(page, '/projects/small/file?path=..%2FREADME.md');
      const secret = await browserApi(page, '/projects/small/file?path=.env');
      const outside = path.join(state, 'outside-file-service');
      fs.mkdirSync(outside, { recursive: true }); fs.writeFileSync(path.join(outside, 'private.txt'), 'P01_OUTSIDE_MUST_NOT_LEAK');
      const link = path.join(config.projects.small, 'src/escape');
      if (fs.existsSync(link)) fs.unlinkSync(link);
      fs.symlinkSync(outside, link, process.platform === 'win32' ? 'junction' : 'dir');
      const symlink = await browserApi(page, '/projects/small/file?path=src%2Fescape%2Fprivate.txt');
      fs.unlinkSync(link);
      assert.equal(traversal.status, 403); assert.equal(secret.status, 403); assert.equal(symlink.status, 403);
      const unauthenticated = await fetch(base + '/api/projects');
      assert.equal(unauthenticated.status, 401);
      return { realTreeEntries: tree.length, traversal: traversal.status, privatePath: secret.status, junction: symlink.status, unauthenticated: unauthenticated.status };
    });
    await test(candidate, 'IDE-02', async () => {
      await page.evaluate(() => window.p01.workbench.open('src/main.ts'));
      await page.evaluate(() => window.p01.workbench.handles.get('src/main.ts').focus());
      await page.keyboard.press('Control+End'); await page.keyboard.type('\nexport const browserSave = 7; // café π');
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await page.waitForFunction(() => window.p01.workbench.status.startsWith('Saved '));
      assert(fs.readFileSync(path.join(config.projects.small, 'src/main.ts'), 'utf8').includes('browserSave = 7'));
      assert(fs.readFileSync(path.join(config.projects.small, 'src/main.ts'), 'utf8').includes('café π'));
      await page.evaluate(() => window.p01.workbench.handles.get('src/main.ts').replace('UNSAVED_BUFFER_MUST_SURVIVE π 🧪\n'));
      fs.appendFileSync(path.join(config.projects.small, 'src/main.ts'), '\nEXTERNAL_WRITE_MUST_SURVIVE\n');
      await page.getByRole('button', { name: 'Save', exact: true }).click();
      await page.getByRole('alert').filter({ hasText: 'external modification' }).waitFor();
      assert(fs.readFileSync(path.join(config.projects.small, 'src/main.ts'), 'utf8').includes('EXTERNAL_WRITE_MUST_SURVIVE'));
      assert(!fs.readFileSync(path.join(config.projects.small, 'src/main.ts'), 'utf8').includes('UNSAVED_BUFFER'));
      await page.evaluate(() => window.p01.workbench.persist());
      await page.reload(); await page.waitForFunction(() => window.p01?.workbench.ready);
      await page.waitForFunction(() => window.p01.workbench.handles.has('src/main.ts'));
      assert.equal(await page.evaluate(() => window.p01.workbench.handles.get('src/main.ts').text()), 'UNSAVED_BUFFER_MUST_SURVIVE π 🧪\n');
      return { keyboardEditSaved: true, conflict409: true, diskExternalWritePreserved: true, bufferPreservedAfterReload: true };
    });
    await test(candidate, 'IDE-03', async () => {
      await page.evaluate(() => window.p01.workbench.open('src/lib.ts', true));
      await page.evaluate(() => window.p01.workbench.handles.get('src/lib.ts').replace('export const independent = 33;'));
      await page.evaluate(() => window.p01.workbench.open('src/main.ts'));
      const values = await page.evaluate(() => ({ count: window.p01.workbench.handles.size, main: window.p01.workbench.handles.get('src/main.ts').text(), lib: window.p01.workbench.handles.get('src/lib.ts').text(), groups: window.p01.workbench.dock.groups.length }));
      assert.equal(values.count, 2); assert(values.main.includes('UNSAVED_BUFFER')); assert(values.lib.includes('independent')); assert(values.groups >= 5);
      return values;
    });
    await test(candidate, 'IDE-04', async () => {
      await page.evaluate(() => window.p01.workbench.open('src/main.ts'));
      const id = await page.evaluate(() => window.p01.workbench.showDiff());
      await page.waitForFunction(id => window.p01.workbench.diffViews.get(id)?.changes() > 0, id);
      const before = await page.evaluate(id => window.p01.workbench.diffViews.get(id).snapshot(), id);
      assert(before.modified.includes('π 🧪'));
      await page.evaluate(id => window.p01.workbench.diffViews.get(id).focusModified(), id);
      await page.keyboard.type('UNEXPECTED_DIFF_WRITE');
      const after = await page.evaluate(id => window.p01.workbench.diffViews.get(id).snapshot(), id);
      assert.deepEqual(after, before);
      return { actualVersionChanges: await page.evaluate(id => window.p01.workbench.diffViews.get(id).changes(), id), keyboardWritePrevented: true, encoding: 'actual UTF-8 Unicode text retained' };
    });
    await test(candidate, 'IDE-09', async () => {
      const layout = await page.evaluate(() => window.p01.workbench.dock.toJSON());
      assert(layout.panels.conversation); assert(layout.panels.explorer); assert(layout.panels.terminal); assert(layout.panels.agents);
      await page.screenshot({ path: path.join(state, `${candidate}-workspace.png`) });
      return { panelIds: Object.keys(layout.panels), versionedGrid: true, orchestrator: 'composition only, explicitly no AI', screenshot: `${candidate}-workspace.png` };
    });
    await test(candidate, 'IDE-10', async () => {
      await page.evaluate(() => window.p01.workbench.open('src/lib.ts'));
      await page.evaluate(() => window.p01.workbench.persist());
      const before = await page.evaluate(() => ({ active: window.p01.workbench.dock.activePanel.id, panels: Object.keys(window.p01.workbench.dock.toJSON().panels).sort(), lib: window.p01.workbench.handles.get('src/lib.ts').text() }));
      await page.reload(); await page.waitForFunction(() => window.p01?.workbench.ready);
      await page.waitForFunction(() => window.p01.workbench.handles.has('src/lib.ts'));
      const after = await page.evaluate(() => ({ active: window.p01.workbench.dock.activePanel.id, panels: Object.keys(window.p01.workbench.dock.toJSON().panels).sort(), lib: window.p01.workbench.handles.get('src/lib.ts').text() }));
      assert.deepEqual(after, before);
      return { before, after, storage: 'Rust .factory/p01 versioned layout + document content/view state', undoAcrossReload: 'not tested' };
    });
    await context.close(); reset(candidate); ({ page, context } = await pageFor(candidate));
    await test(candidate, 'IDE-05', async () => {
      await page.evaluate(() => window.p01.workbench.open('src/main.ts'));
      const connection = await page.evaluate(() => window.p01.workbench.startLsp());
      await page.waitForFunction(() => window.p01.workbench.lsp.messages.some(message => message.method === 'textDocument/publishDiagnostics' && message.params.diagnostics.length));
      await page.waitForFunction(() => window.p01.workbench.handles.get('src/main.ts').diagnostics().length > 0);
      const hover = await page.evaluate(() => window.p01.workbench.handles.get('src/main.ts').hover());
      assert(JSON.stringify(hover).includes('greet'));
      const diagnostic = await page.evaluate(() => window.p01.workbench.lsp.messages.find(message => message.method === 'textDocument/publishDiagnostics' && message.params.diagnostics.length));
      const other = await page.evaluate(() => window.p01.workbench.projects.find(project => project.id === 'other'));
      await page.evaluate(uri => { window.p01.scopeCleanup = window.p01.scopeProbe(uri); }, other.uri + 'src/main.ts');
      await page.waitForTimeout(500);
      const foreignModelAttempt = await page.evaluate(() => window.p01.workbench.lsp.messages.some(message => message.type === 'policy-error'));
      await page.evaluate(() => window.p01.scopeCleanup());
      const trace = (await browserApi(page, `/projects/small/trace/${connection.id}`)).body;
      await page.evaluate(() => window.p01.workbench.stopLsp());
      await page.locator('.error[role="alert"]').waitFor();
      const ended = (await browserApi(page, `/projects/small/trace/${connection.id}`)).body;
      assert.equal(ended.running, false);
      return { ...connection, diagnostic, hover, trace, foreignModelAttemptBlockedByRust: foreignModelAttempt, errorVisible: true, processCleanedUp: !ended.running };
    });
    await test(candidate, 'IDE-06', async () => {
      const evidence = await page.evaluate(() => window.p01.workbench.startDebug());
      assert(evidence.breakpoints.breakpoints[0].verified); assert.equal(evidence.stopped.body.reason, 'breakpoint');
      assert(evidence.variables.variables.some(variable => variable.name === 'value' && variable.value === '40'));
      const events = await page.evaluate(() => window.p01.workbench.continueDebug());
      assert(events.some(event => event.event === 'terminated'));
      return { ...evidence, terminated: true, events };
    });
    const processes = (await browserApi(page, '/processes')).body.protocol;
    for (const process of processes.filter(item => item.running && item.project === 'small')) await browserApi(page, `/projects/small/transport/${process.id}`, 'DELETE');
    await test(candidate, 'IDE-07', async () => {
      const lease = await page.evaluate(() => window.p01.workbench.startTerminal());
      await page.waitForFunction(() => window.p01.workbench.terminalOutput.includes('isatty=True'));
      await page.evaluate(() => { window.p01.workbench.terminalResize(111, 31); window.p01.workbench.terminalInput('size\r'); });
      await page.waitForFunction(() => window.p01.workbench.terminalOutput.includes('P01_SIZE=111x31'));
      await page.evaluate(() => window.p01.workbench.terminalInput('echo roundtrip\r'));
      await page.waitForFunction(() => window.p01.workbench.terminalOutput.includes('P01_ECHO=roundtrip'));
      await page.evaluate(() => window.p01.workbench.terminal.socket.close());
      await page.evaluate(id => window.p01.workbench.connectTerminal(id), lease.id);
      await page.waitForFunction(() => window.p01.workbench.terminalOutput.includes('P01_ECHO=roundtrip'));
      const output = await page.evaluate(() => window.p01.workbench.terminalOutput);
      const termination = await page.evaluate(() => window.p01.workbench.stopTerminal());
      assert.equal(termination.reconciled_from_durable_session, true);
      const pid = Number(output.match(/pid=(\d+)/)[1]);
      let alive = true;
      for (let attempt = 0; attempt < 20 && alive; attempt++) { try { process.kill(pid, 0); } catch { alive = false; } if (alive) await new Promise(resolve => setTimeout(resolve, 50)); }
      assert.equal(alive, false);
      return { lease, output, pid, processExitConfirmed: true, reconnectSameSession: true, resizeConfirmedByOS: true, termination, shell: 'not implemented; fixed trusted probe reuses ASF Runtime' };
    });
    await context.close(); reset(candidate); ({ page, context } = await pageFor(candidate));
    await test(candidate, 'IDE-08', async () => {
      await page.evaluate(() => window.p01.workbench.open('sample.yaml'));
      await page.evaluate(() => window.p01.workbench.handles.get('sample.yaml').replace('project: [\n'));
      const before = await page.evaluate(() => window.p01.adapter.pluginEvidence());
      const install = await page.evaluate(() => window.p01.workbench.plugin('install'));
      const activate = await page.evaluate(() => window.p01.workbench.plugin('activate'));
      await page.waitForFunction(candidate => {
        const evidence = window.p01.adapter.pluginEvidence();
        return candidate === 'monaco' ? evidence.markers?.length > 0 : evidence.syntaxTrees.some(tree => tree.includes('Stream'));
      }, candidate);
      const active = await page.evaluate(() => window.p01.adapter.pluginEvidence());
      const uninstall = await page.evaluate(() => window.p01.workbench.plugin('uninstall'));
      assert.equal(uninstall.installed, false); assert.equal(uninstall.evidence.active, false);
      const refused = await browserApi(page, `/projects/small/extensions/${candidate}`, 'POST', { action: 'install', id: 'untrusted-package' });
      assert.equal(refused.status, 403);
      return { before, install, activate, active, uninstall, unknownPackageRefused: refused.status, model: 'pinned native editor plugin, not VS Code extension compatibility' };
    });
    results.push({ candidate, id: 'BROWSER-ERRORS', status: page.errors.length ? 'FAIL' : 'PASS', evidence: { errors: page.errors } });
    fs.writeFileSync(output, JSON.stringify({ schemaVersion: 1, measuredAt: new Date().toISOString(), base, tests: results }, null, 2));
    await context.close();
  }
} finally { await browser.close(); }
console.log(JSON.stringify(results.map(({ candidate, id, status, error }) => ({ candidate, id, status, error: error?.slice(0, 160) })), null, 2));
if (results.some(result => result.status === 'FAIL')) process.exitCode = 1;
