import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const local = path.join(root, '.factory/v2/p08-r1');
const output = process.env.ASF_P08_R1_REPLAY_OUTPUT ?? path.join(local, 'replay');
await fs.mkdir(path.join(output, 'screenshots'), {recursive: true});
try {await fs.access(path.join(output, 'results.json')); throw new Error('Prior replay retained; choose a new output.');} catch (error) {if (error.code !== 'ENOENT') throw error;}
const binaryRecord = JSON.parse((await fs.readFile(path.join(local, 'binary.json'), 'utf8')).replace(/^\uFEFF/, ''));
const binary = binaryRecord.binary;
const styleProbe = process.env.ASF_P08_R1_STYLE_PROBE === '1';
const hash = value => createHash('sha256').update(value).digest('hex');
assert.equal(hash(await fs.readFile(binary)), binaryRecord.sha256);
const accepted = JSON.parse(await fs.readFile(path.join(local, 'acceptance-semantic-direct/results.json'), 'utf8'));
const interrupted = JSON.parse(await fs.readFile(path.join(local, 'acceptance-interruptions/results.json'), 'utf8'));
assert.ok(accepted.scenarios.Semantic.qualified && interrupted.scenarios.Cancel.qualified && interrupted.scenarios.Restart.qualified);
const require = createRequire(path.join(root, 'apps/dashboard/package.json'));
const {chromium} = require('playwright');
const result = {schema: 1, kind: styleProbe ? 'DEVELOPMENT CSS PROBE ONLY; not final embedded application qualification' : 'Model-free browser inspection of actual durable real-agent results', binarySha256: binaryRecord.sha256, started: new Date().toISOString(), tests: [], screenshots: [], errors: [], assets: []};
const persist = () => fs.writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2) + '\n');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const browser = await chromium.launch({channel: 'msedge', headless: true});
result.browser = browser.version();
let child, context, page, base, csrf;
async function until(fn, label) {
  const end = Date.now() + 60000;
  while (Date.now() < end) {const value = await fn(); if (value) return value; await delay(200);}
  throw new Error('Timeout: ' + label);
}
async function start(record) {
  const port = await new Promise(resolve => {const server = net.createServer(); server.listen(0, '127.0.0.1', () => {const port = server.address().port; server.close(() => resolve(port));});});
  base = `http://127.0.0.1:${port}`;
  const handle = await fs.open(path.join(output, `server-${Date.now()}.log`), 'w');
  child = spawn(binary, ['start', '--no-browser', '--port', String(port), '--registry', path.join(record.fixture, 'registry.sqlite3'), '--allow-project-root', record.fixture], {cwd: record.scenarios.Semantic.root, windowsHide: true, stdio: ['ignore', handle.fd, handle.fd]});
  await handle.close();
  await until(async () => {try {return (await fetch(base + '/api/health')).ok;} catch {return false;}}, 'backend');
  context = await browser.newContext({viewport: {width: 1680, height: 1050}, reducedMotion: 'reduce', colorScheme: 'dark'});
  page = await context.newPage();
  page.on('pageerror', error => result.errors.push(String(error)));
  const response = await context.request.post(base + '/api/auth/session', {headers: {Origin: base}, data: {}});
  assert.equal(response.status(), 200);
  csrf = (await response.json()).csrfToken;
  for (const item of Object.values(record.scenarios).filter(item => item.project)) {
    const response = await context.request.post(`${base}/api/projects/${item.project}/open`, {headers: {Origin: base, 'X-Factory-CSRF': csrf}, data: {}});
    assert.equal(response.status(), 200, 'Registered project reopens with its original identity');
  }
  await page.goto(base + '/workspace.html');
  await page.getByLabel('Open registered project').waitFor();
}
async function stop() {
  await context?.close();
  if (child && child.exitCode === null && child.signalCode === null) {child.kill(); await new Promise(resolve => child.once('exit', resolve));}
}
async function get(item, suffix) {
  const response = await context.request.get(`${base}/api/projects/${item.project}/${suffix}`);
  assert.ok(response.ok(), 'Authorized project-scoped read succeeds');
  return response.json();
}
async function view(name) {await page.getByRole('button', {name: 'Views', exact: true}).click(); await page.getByRole('menuitem', {name, exact: true}).click();}
async function select(item) {
  await page.getByLabel('Open registered project').selectOption(item.project);
  await view('Orchestrator');
  await page.locator('.chat-panel:visible').first().getByLabel('Conversation', {exact: true}).selectOption(item.conversation);
  await view('Progress');
}
async function shot(name) {await page.screenshot({path: path.join(output, 'screenshots', name + '.png'), fullPage: true}); result.screenshots.push(name);}
async function test(name, fn) {
  const start = performance.now();
  try {await fn(); result.tests.push({name, status: 'passed', ms: performance.now() - start}); console.log('PASS replay ' + name);}
  catch (error) {result.tests.push({name, status: 'failed', error: String(error).split('\n')[0]}); throw error;}
  finally {await persist();}
}
const saved = () => page.locator('.ws-status').getByRole('status').filter({hasText: /^Workspace saved$/}).waitFor();
async function geometry() {
  return page.evaluate(() => ({
    viewport: {width: innerWidth, height: innerHeight, scrollX, scrollY},
    active: document.activeElement?.className,
    elements: ['.ws-app', '.ws-main', '.ws-workarea', '.ws-dock', '.files-editor', '.cm-editor'].flatMap(selector => [...document.querySelectorAll(selector)].map(element => {
      const rect = element.getBoundingClientRect(), style = getComputedStyle(element);
      return {selector, x: rect.x, y: rect.y, width: rect.width, height: rect.height, scrollTop: element.scrollTop, scrollLeft: element.scrollLeft, clientHeight: element.clientHeight, scrollHeight: element.scrollHeight, overflow: style.overflow};
    })),
  }));
}
try {
  await start(accepted);
  const item = accepted.scenarios.Semantic;
  await test('Actual previous and revised plans, task failure and published integration are inspectable', async () => {
    await select(item);
    const progress = page.locator('.chat-mission:visible').first();
    await progress.locator('details[aria-label="Plan revision history"] > summary').click();
    assert.match(await progress.innerText(), /Revision 1/);
    assert.match(await progress.innerText(), /Revision 2/);
    assert.match(await progress.innerText(), /superseded/);
    await progress.locator('details[aria-label="Integration history"] > summary').click();
    assert.match(await progress.innerText(), new RegExp(item.mission.integration_sha));
    await shot('semantic-plan-history-and-integration');
    await progress.locator('summary').filter({hasText: /^Tasks, agents and evidence$/}).click();
    assert.match(await progress.innerText(), /Researcher process exited with code 1/);
    assert.match(await progress.innerText(), /Review: approve/);
    await shot('semantic-tests-reviews-and-failure-history');
  });
  await test('Actual source diff and genuine review decision open in the existing workflow view', async () => {
    await page.getByRole('button', {name: 'Open workflow and review', exact: true}).first().click();
    await page.getByRole('button', {name: 'Layout actions', exact: true}).click();
    await page.getByRole('menuitem', {name: 'Toggle maximization', exact: true}).click();
    const attempt = page.locator('.ws-attempt-evidence:visible').filter({hasText: /approved/}).first();
    await attempt.locator('summary').click();
    await attempt.locator('p').filter({hasText: /^Reviewer: approve/}).first().waitFor();
    assert.match(await attempt.innerText(), /Reviewer: approve/);
    const patch = await attempt.locator('pre').innerText();
    assert.match(patch, /src\/intervals\.py/);
    assert.match(patch, /max\(/);
    assert.match(patch, /tests\/test_intervals\.py/);
    assert.match(await attempt.innerText(), /Rust|Patch SHA-256/);
    await shot('semantic-captured-patch-and-review');
    await attempt.locator('pre').scrollIntoViewIfNeeded();
    await shot('semantic-source-diff');
    await page.getByRole('button', {name: 'Layout actions', exact: true}).click();
    await page.getByRole('menuitem', {name: 'Toggle maximization', exact: true}).click();
  });
  await test('Correct integration snapshot opens read-only in a wide CodeMirror group', async () => {
    await page.getByRole('button', {name: 'Code', exact: true}).click();
    await view('Explorer');
    const roots = (await get(item, 'files/roots')).roots;
    const integration = roots.find(value => value.kind === 'integration_snapshot');
    assert.ok(integration?.read_only);
    const explorer = page.locator('.files-explorer:visible').first();
    await explorer.getByLabel('Filesystem root', {exact: true}).selectOption(integration.id);
    const tree = explorer.getByRole('tree');
    const directory = tree.getByRole('treeitem').filter({has: page.locator('.files-tree-name', {hasText: /^src$/})}).first();
    if ((await directory.getAttribute('aria-expanded')) !== 'true') await directory.click();
    await tree.getByRole('treeitem').filter({has: page.locator('.files-tree-name', {hasText: /^intervals\.py$/})}).first().click();
    const editor = page.locator('.files-editor:visible').last();
    await editor.locator('.cm-content').waitFor();
    assert.equal(await editor.locator('.cm-content').getAttribute('contenteditable'), 'false');
    assert.match(await editor.locator('.cm-content').innerText(), /max\(/);
    await saved();
    await shot('semantic-wide-integrated-editor-dark');
    await page.getByLabel('Theme', {exact: true}).selectOption('light');
    await saved();
    await shot('semantic-wide-integrated-editor-light');
    await page.reload();
    await page.getByLabel('Open registered project').waitFor();
    await saved();
    assert.match(await page.locator('.files-editor:visible').last().innerText(), /max\(/);
    assert.equal((await get(item, `conversations/${item.conversation}/missions`))[0].status, 'completed');
    await page.setViewportSize({width: 1024, height: 768});
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    result.geometryAfterResize = await geometry();
    await persist();
    if (styleProbe) {
      await page.addStyleTag({content: '.ws-dock { overflow: clip; }'});
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      result.geometryAfterDevelopmentStyle = await geometry();
    }
    assert.equal(await page.locator('.ws-dock').evaluate(element => element.scrollTop), 0, 'Dock host must not scroll its panels outside the viewport');
    await until(async () => {
      const box = await page.locator('.files-editor:visible').last().boundingBox();
      return box && box.y >= 70 && box.height >= 200 && box.y + box.height <= 768;
    }, 'narrow editor reflow');
    await saved();
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await shot('semantic-narrow-restored');
  });
  if (styleProbe) result.tests.push({name: 'Final served assets match the built embedded dashboard', status: 'not_executed', reason: 'Development CSS probe only; final embedded qualification must execute separately without style injection'});
  else await test('Final served assets match the built embedded dashboard', async () => {
    const dist = path.join(root, 'apps/dashboard/dist');
    async function visit(folder) {
      for (const entry of await fs.readdir(folder, {withFileTypes: true})) {
        const file = path.join(folder, entry.name);
        if (entry.isDirectory()) await visit(file);
        else {
          const relative = path.relative(dist, file).replaceAll('\\', '/');
          const response = await context.request.get(base + '/' + relative);
          assert.equal(response.status(), 200);
          const expected = hash(await fs.readFile(file));
          assert.equal(hash(await response.body()), expected, relative);
          result.assets.push({path: relative, sha256: expected});
        }
      }
    }
    await visit(dist);
  });
  await stop();
  await start(interrupted);
  await test('Cancelled and interrupted conversations restore in their owning projects without replay', async () => {
    for (const name of ['Cancel', 'Restart']) {
      const item = interrupted.scenarios[name];
      await select(item);
      const before = await get(item, `runs/${item.mission.run_id}`);
      await page.reload();
      await page.getByLabel('Open registered project').waitFor();
      await select(item);
      const after = await get(item, `runs/${item.mission.run_id}`);
      assert.equal(after.sessions.length, before.sessions.length);
      assert.ok(after.sessions.every(value => value.status === (name === 'Cancel' ? 'cancelled' : 'interrupted')));
      const state = await get(item, `runs/${item.mission.run_id}/scheduler`);
      assert.equal(state.activeClaims, 0);
      assert.equal(state.ownerActive, false);
      await saved();
      await shot(name.toLowerCase() + '-durable-recovered-conversation');
    }
    const a = interrupted.scenarios.Cancel, b = interrupted.scenarios.Restart;
    const denied = await context.request.get(`${base}/api/projects/${a.project}/conversations/${b.conversation}`);
    assert.equal(denied.status(), 404);
  });
  assert.deepEqual(result.errors, []);
  result.status = 'PASS';
} catch (error) {
  result.status = 'FAIL';
  result.failure = String(error).split('\n')[0];
  result.failureGeometry = await geometry().catch(() => null);
  console.error(result.failure);
  process.exitCode = 1;
  if (page) await shot('failure').catch(() => {});
} finally {result.finished = new Date().toISOString(); await persist(); await stop(); await browser.close();}
