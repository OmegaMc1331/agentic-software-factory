import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const output = process.env.ASF_P08_R1_OUTPUT ?? path.join(root, '.factory/v2/p08-r1/acceptance');
const binary = process.env.ASF_P08_R1_BINARY ?? path.join(root, '.factory/v2/p08/run/factory-p08-final-8b249b4.exe');
const cli = process.env.ASF_P08_R1_CODEX ?? 'codex';
const mode = process.env.ASF_P08_R1_MODE ?? 'all';
assert.ok(['all', 'semantic', 'interruptions'].includes(mode));
const require = createRequire(path.join(root, 'apps/dashboard/package.json'));
const {chromium} = require('playwright');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const safeError = error => String(error).replace(/(?:cookie|x-factory-csrf):[^\r\n]*/gi, '[authentication header redacted]');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const recordFile = path.join(output, 'results.json');
await fs.mkdir(path.join(output, 'screenshots'), {recursive: true});
let prior;
try { prior = JSON.parse(await fs.readFile(recordFile, 'utf8')); } catch (error) { if (error.code !== 'ENOENT') throw error; }
if (prior && process.env.ASF_P08_R1_RESUME !== '1') throw new Error('Prior acceptance retained. Explicit resume or a new output is required.');
const result = prior ?? {
  schema: 1, started: new Date().toISOString(),
  fixture: await fs.mkdtemp(path.join(os.tmpdir(), 'asf-p08-r1-')),
  machine: {platform: os.platform(), release: os.release(), cpu: os.cpus()[0].model, cpus: os.cpus().length, ram: os.totalmem()},
  scenarios: {}, tests: [], screenshots: [], pageErrors: [], serverTimeline: [],
};
const binaryHash = hash(await fs.readFile(binary));
if (prior) assert.equal(prior.binarySha256, binaryHash);
Object.assign(result, {binary, binarySha256: binaryHash, scriptSha256: hash(await fs.readFile(fileURLToPath(import.meta.url))), mode});
const persist = () => fs.writeFile(recordFile, JSON.stringify(result, null, 2) + '\n');
function run(program, args, cwd, timeout = 60000) {
  const begin = performance.now();
  const value = spawnSync(program, args, {cwd, encoding: 'utf8', timeout, windowsHide: true, env: {...process.env, PYTHONDONTWRITEBYTECODE: '1'}});
  return {program, args, cwd, exit: value.status, stdout: value.stdout ?? '', stderr: value.stderr ?? '', error: value.error?.message, ms: performance.now() - begin};
}
function ok(value) { assert.equal(value.exit, 0, JSON.stringify(value)); return value.stdout.trim(); }
result.sourceCommit = ok(run('git', ['rev-parse', 'HEAD'], root));
result.cliVersion = ok(run(cli, ['--version'], root));
const intervalSource = `def merge_intervals(intervals):
    """Return sorted, disjoint closed intervals; merge overlapping boundaries."""
    ordered = sorted((start, end) for start, end in intervals)
    merged = []
    for start, end in ordered:
        if start > end:
            raise ValueError("interval start must not exceed end")
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], end)
        else:
            merged.append((start, end))
    return merged
`;
const intervalTests = `import unittest
from src.intervals import merge_intervals

class IntervalTests(unittest.TestCase):
    def test_empty(self):
        self.assertEqual(merge_intervals([]), [])
    def test_disjoint_and_sorting(self):
        self.assertEqual(merge_intervals([(5, 7), (1, 2)]), [(1, 2), (5, 7)])
    def test_touching(self):
        self.assertEqual(merge_intervals([(1, 3), (3, 5)]), [(1, 5)])
    def test_nested_then_contained_range(self):
        self.assertEqual(merge_intervals([(1, 10), (2, 3), (4, 5)]), [(1, 10)])
    def test_reversed_range(self):
        with self.assertRaises(ValueError):
            merge_intervals([(7, 3)])
    def test_input_unchanged(self):
        source = [(4, 6), (1, 2)]
        merge_intervals(source)
        self.assertEqual(source, [(4, 6), (1, 2)])
`;
const verifier = `import json,pathlib,subprocess,sys,time
mode=sys.argv[1]
prompt=sys.stdin.read()
if mode=='verify':
    check=subprocess.run([sys.executable,'-B','-m','unittest','discover','-s','tests','-v'],capture_output=True,text=True)
    sys.stderr.write(check.stdout+check.stderr)
    if check.returncode: sys.exit(check.returncode)
    print(json.dumps({'summary':'Actual unittest suite passed','findings':[check.stderr], 'recommendations':[]}))
elif mode=='hold':
    print('CONTROLLED_PROCESS_RUNNING',flush=True)
    time.sleep(600)
    sys.exit(8)
else: raise ValueError(mode)
`;
function profiles(folder, name) {
  const controlled = name === 'Control';
  let text = `[runtime]\nmax_parallel_tasks = 2\n[scheduler]\nmax_parallel_tasks = 2\nmax_project_tasks = 2\nmax_replans = ${name === 'Semantic' ? 1 : 0}\nmax_total_tasks = 16\nmax_duration_secs = 1200\n[[integration_checks]]\nprogram = "python"\nargs = ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"]\ntimeout_secs = 30\n`;
  for (const role of ['planner', 'worker', 'reviewer']) {
    const args = ['exec', '--ignore-user-config', '--ephemeral', '--disable', 'apps', '--sandbox', role === 'worker' ? 'workspace-write' : 'read-only', '-c', 'windows.sandbox=mxc', '-c', 'allow_login_shell=false', '--color', 'never'];
    text += `\n[agents.${role}]\n${controlled ? '' : 'kind = "codex"\n'}command = ${JSON.stringify(controlled ? 'python' : cli)}\nargs = ${JSON.stringify(controlled ? [path.join(folder, '.factory', 'control.py'), 'hold'] : args)}\nmax_concurrency = 1\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE = "1"\n[[role_assignments]]\nrole = "${role}"\nagent = "${role}"\npreferred = true\n`;
  }
  if (name === 'Semantic') text += `\n[agents.verifier]\ncommand = "python"\nargs = ${JSON.stringify([path.join(folder, '.factory', 'control.py'), 'verify'])}\nmax_concurrency = 1\n[[role_assignments]]\nrole = "researcher"\nagent = "verifier"\npreferred = true\n`;
  for (const role of ['planner', 'worker', 'reviewer', 'researcher']) text += `\n[policies.roles.${role}.filesystem]\nread = ["**"]\nwrite = ${role === 'worker' ? '["src/**", "tests/**"]' : '[]'}\n[policies.roles.${role}.commands]\nmode = "restricted"\nallow = ["python", "python3", "git", "rg", "Get-Content", "Set-Content", "Get-ChildItem", "Get-Location", "Test-Path", "powershell", "powershell.exe"]\ndeny = ["curl", "Invoke-WebRequest", "cmd", "bash", "sh"]\n[policies.roles.${role}.network]\nmode = "deny"\n`;
  return text;
}
async function prepare(name) {
  if (result.scenarios[name]) return result.scenarios[name];
  const folder = path.join(result.fixture, name);
  await fs.mkdir(path.join(folder, 'src'), {recursive: true});
  await fs.mkdir(path.join(folder, 'tests'), {recursive: true});
  const files = {'.gitignore': '.factory/\n__pycache__/\n*.pyc\n', 'src/__init__.py': '', 'src/intervals.py': intervalSource, 'tests/test_intervals.py': intervalTests};
  for (const [file, content] of Object.entries(files)) await fs.writeFile(path.join(folder, file), content);
  const baseline = run('python', ['-B', '-m', 'unittest', 'discover', '-s', 'tests', '-v'], folder);
  assert.equal(baseline.exit, 1);
  assert.match(baseline.stderr, /test_nested_then_contained_range.*FAIL/s);
  await fs.writeFile(path.join(folder, 'tests/original_failure.json'), JSON.stringify({schema: 1, sourceSha256: hash(Buffer.from(intervalSource)), suiteSha256: hash(Buffer.from(intervalTests)), observedAt: new Date().toISOString(), qualification: 'Actual harness execution before any agent invocation or source modification. Immutable evidence, not a proposed patch.', ...baseline}, null, 2) + '\n');
  ok(run('git', ['init', '-q', '-b', 'main'], folder));
  ok(run('git', ['config', 'user.name', 'ASF P08 R1'], folder));
  ok(run('git', ['config', 'user.email', 'p08-r1@example.invalid'], folder));
  ok(run('git', ['add', '.'], folder));
  ok(run('git', ['commit', '-qm', 'reproducible interval behavior fixture'], folder));
  ok(run(binary, ['init'], folder));
  await fs.writeFile(path.join(folder, '.factory/config.toml'), profiles(folder, name));
  await fs.writeFile(path.join(folder, '.factory/control.py'), verifier);
  const item = {root: folder, initialHead: ok(run('git', ['rev-parse', 'HEAD'], folder)), baselineBlob: ok(run('git', ['rev-parse', 'HEAD:tests/original_failure.json'], folder)), baseline, samples: []};
  assert.equal(ok(run('git', ['remote'], folder)), '');
  result.scenarios[name] = item;
  await persist();
  return item;
}
for (const name of ['Semantic', 'Cancel', 'Restart', 'Control']) await prepare(name);
let child, browser, context, page, csrf;
let base = '';
async function eventually(fn, label, ms = 60000, interval = 200) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const value = await fn(); if (value) return value; await delay(interval); }
  throw new Error(`Timeout: ${label}`);
}
async function startServer() {
  const port = await new Promise(resolve => {const socket = net.createServer(); socket.listen(0, '127.0.0.1', () => {const value = socket.address().port; socket.close(() => resolve(value));});});
  base = `http://127.0.0.1:${port}`;
  const logfile = `server-${Date.now()}.log`;
  const handle = await fs.open(path.join(output, logfile), 'w');
  child = spawn(binary, ['start', '--no-browser', '--port', String(port), '--max-active-tasks', '2', '--registry', path.join(result.fixture, 'registry.sqlite3'), '--allow-project-root', result.fixture], {cwd: result.scenarios.Semantic.root, windowsHide: true, stdio: ['ignore', handle.fd, handle.fd]});
  await handle.close();
  result.serverTimeline.push({event: 'started', at: new Date().toISOString(), pid: child.pid, base, logfile});
  await eventually(async () => {try {return (await fetch(base + '/api/health')).ok;} catch {return false;}}, 'owned server ready');
  await persist();
}
async function stopServer(reason) {
  const owned = child;
  if (owned && owned.exitCode === null && owned.signalCode === null) {
    result.serverTimeline.push({event: reason, at: new Date().toISOString(), pid: owned.pid});
    owned.kill();
    await new Promise(resolve => owned.once('exit', resolve));
  }
  await persist();
}
async function auth() {
  const response = await context.request.post(base + '/api/auth/session', {headers: {Origin: base}, data: {}});
  assert.ok(response.ok(), await response.text());
  csrf = (await response.json()).csrfToken;
}
async function mutate(url, data, expected = 200, method = 'POST') {
  if (!csrf) await auth();
  const response = await context.request.fetch(base + url, {method, data, headers: {Origin: base, 'X-Factory-CSRF': csrf}});
  assert.equal(response.status(), expected, await response.text());
  return expected === 204 ? null : response.json();
}
async function get(item, suffix) {
  const response = await context.request.get(`${base}/api/projects/${item.project}/${suffix}`);
  assert.ok(response.ok(), await response.text());
  return response.json();
}
async function register() {
  const response = await context.request.get(base + '/api/projects');
  const registered = await response.json();
  const canonical = value => value.replace(/^\\\\\?\\/, '').replaceAll('\\', '/').toLowerCase();
  for (const [name, item] of Object.entries(result.scenarios)) {
    const entry = registered.find(value => canonical(value.canonical_root) === canonical(item.root));
    if (entry) item.project = entry.project_id;
    if (!item.project) item.project = (await mutate('/api/projects/register', {path: item.root, display_name: `P08 R1 ${name}`, trust_project: true}, 201)).project_id;
    await mutate(`/api/projects/${item.project}/open`, {});
  }
  await persist();
}
async function shot(name) {
  await page.screenshot({path: path.join(output, 'screenshots', name + '.png'), fullPage: true});
  if (!result.screenshots.includes(name)) result.screenshots.push(name);
}
async function replay(item) {
  const stream = await page.evaluate(async url => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 10000);
    try {
      const response = await fetch(url, {signal: controller.signal});
      const reader = response.body.getReader(), decoder = new TextDecoder();
      let text = '';
      while (text.length < 65536 && !text.includes('event: conversation')) {
        const chunk = await reader.read();
        if (chunk.done) break;
        text += decoder.decode(chunk.value, {stream: true});
      }
      await reader.cancel();
      return {status: response.status, text};
    } finally {clearTimeout(timer); controller.abort();}
  }, `${base}/api/projects/${item.project}/conversations/${item.conversation}/events?after=0`);
  assert.equal(stream.status, 200);
  assert.match(stream.text, /event: conversation/);
  const events = [...stream.text.matchAll(/^data: (.+)$/gm)].map(value => JSON.parse(value[1]));
  assert.ok(events.length);
  assert.ok(events.every(value => value.conversation_id === item.conversation));
  assert.equal(new Set(events.map(value => value.id)).size, events.length);
  for (let i = 1; i < events.length; i++) assert.ok(events[i].id > events[i - 1].id);
  item.replay = {at: new Date().toISOString(), status: stream.status, events};
}
const panel = () => page.locator('.chat-panel:visible').first();
async function view(label) {await page.getByRole('button', {name: 'Views', exact: true}).click(); await page.getByRole('menuitem', {name: label, exact: true}).click();}
async function select(item) {
  await page.getByLabel('Open registered project').selectOption(item.project);
  await view('Orchestrator');
  if (item.conversation) await panel().getByLabel('Conversation', {exact: true}).selectOption(item.conversation);
}
const objective = 'Investigate the incorrect interval-handling behavior in this project. Identify the root cause, fix the implementation, and add regression tests for the affected edge cases. Preserve valid existing behavior. Run the test suite and verify the correction. Intervals are closed; touching and overlapping ranges merge, including nested and negative ranges. Input must remain unchanged, reversed ranges raise ValueError. The initial diagnostic task must record the real failing suite before changes. If it fails, use the existing bounded recovery Planner to replace unfinished work with a minimal repair task. Do not retry the same diagnostic as a substitute for fixing the defect. The harness has retained the actual original six-test suite output, exit status, traceback, timestamp and source hashes in tests/original_failure.json before any agent invocation. Read and cite that immutable original evidence; never edit or delete it. It is not a reconstructed run or a proposed patch. ASF owns review, commits and integration. Only src/** and tests/** may change. No publication, downloads, policy changes, broad approvals, staging or commits. Obey the existing command policy and native sandbox; do not bypass a denial. This restricted Windows policy permits direct executable invocations, not shell scripts: use individual listed programs such as Get-Content, git and python. Do not execute PowerShell variable assignments, here-strings, pipelines, shell wrappers or command bundles. Use an already-allowed direct Python invocation for scoped file edits. This instruction neither authorizes an otherwise denied operation nor permits hiding a rejection. Report every executed command as its exact unannotated executable invocation; explanations and exit results belong in summary/results.';
const interruptionObjective = 'Investigate and correct interval merging, preserving valid behavior, with comprehensive regression tests. Work only in src/** and tests/**. First create a meaningful tests/test_additional_intervals.py covering nested intervals, negative endpoints, duplicates and boundary cases from your analysis, then run the tests, diagnose the implementation and fix it. The new tests must exercise real behavior, not be a marker or placeholder. Preserve original tests. Do not stage, commit, publish, download, request broader permissions or bypass a denial. ASF owns review and integration. Use individual commands. This is an authorized disposable coding task.';
async function submit(item, semantic = false, controlled = false) {
  if (item.conversation) return;
  await select(item);
  const creation = page.waitForResponse(value => value.request().method() === 'POST' && /\/conversations$/.test(new URL(value.url()).pathname));
  await panel().getByRole('button', {name: 'New conversation', exact: true}).click();
  item.conversation = (await (await creation).json()).id;
  item.objective = semantic ? objective : controlled ? 'Controlled cross-project process-lifecycle sentinel; no genuine model claim.' : interruptionObjective;
  const tasks = semantic ? [
    {title: 'Record original semantic failure', objective: 'Read-only diagnostic: run the original unittest suite and retain the exact failing behavior. Do not edit files or report success when tests fail.', role: 'researcher', operation: 'advisory', dependencies: []},
    {title: 'Repair interval behavior', objective: 'Diagnose and fix the interval defect, add regression tests, preserve behavior and run all tests. Never provide a predetermined patch.', role: 'worker', operation: 'implement', dependencies: ['T1']},
  ] : [{title: controlled ? 'Owned control process' : 'Implement interval correction', objective: item.objective, role: 'worker', operation: 'implement', dependencies: []}];
  item.submittedAt = new Date().toISOString();
  await panel().getByRole('button', {name: 'Visual plan', exact: true}).click();
  await panel().getByLabel('Mission objective').fill(item.objective);
  await panel().getByLabel('Scope', {exact: true}).fill('src/** and tests/** only');
  await panel().getByLabel('Constraints', {exact: true}).fill('Preserve sandbox and existing policy. No publication. No approval bypass.');
  await panel().locator('summary').filter({hasText: 'Preferred configured agents'}).click();
  await panel().getByLabel('Preferred worker', {exact: true}).selectOption('worker');
  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i], id = `T${i + 1}`;
    await panel().getByRole('button', {name: 'Add task', exact: true}).click();
    await panel().getByLabel(`Title ${id}`, {exact: true}).fill(task.title);
    await panel().getByLabel(`Objective ${id}`, {exact: true}).fill(task.objective);
    await panel().getByLabel(`Role ${id}`, {exact: true}).selectOption(task.role);
    await panel().getByLabel(`Operation ${id}`, {exact: true}).selectOption(task.operation);
    await panel().getByLabel(`Dependencies ${id}`, {exact: true}).fill(task.dependencies.join(','));
    await panel().getByLabel(`Acceptance ${id}`, {exact: true}).fill('Run the actual tests; retain honest evidence; respect the authorized file scope');
  }
  await shot(semantic ? 'semantic-authored-diagnostic-plan' : controlled ? 'control-plan' : 'interruption-plan');
  const pending = page.waitForResponse(value => value.request().method() === 'POST' && /\/messages$/.test(new URL(value.url()).pathname));
  await panel().getByRole('button', {name: 'Validate and launch plan', exact: true}).click();
  const response = await pending;
  assert.equal(response.status(), 202, await response.text());
  item.submission = await response.json();
  await persist();
  await view('Progress');
}
async function snapshot(item) {
  const missions = await get(item, `conversations/${item.conversation}/missions`);
  if (!missions.length) {
    const messages = await get(item, `conversations/${item.conversation}/messages`);
    const failed = messages.find(value => value.role === 'orchestrator' && value.status === 'failed');
    if (failed) throw new Error(`Mission submission failed before dispatch: ${failed.content}`);
    return null;
  }
  item.mission = missions[0];
  item.detail = await get(item, `runs/${item.mission.run_id}`);
  item.scheduler = await get(item, `runs/${item.mission.run_id}/scheduler`);
  item.samples.push({at: new Date().toISOString(), status: item.mission.status, revision: item.mission.plan_revision, claims: item.scheduler.activeClaims, sessions: item.detail.sessions.map(value => ({id: value.id, role: value.role, status: value.status, pid: value.outputMeta?.processId})), tasks: item.mission.tasks.map(value => ({id: value.id, state: value.state}))});
  if (item.detail.attempts.some(value => value.error?.includes('AGENT_RESOURCE_EXHAUSTED')) || item.detail.sessions.some(value => value.status === 'failed' && value.stderr?.startsWith('OpenAI Codex v') && /ERROR: You[’']ve hit your usage limit\./.test(value.stderr))) {
    result.providerRefused = {at: new Date().toISOString(), project: item.project, run: item.mission.run_id};
    await persist();
    throw new Error('MODEL ACCEPTANCE BLOCKED — PROVIDER USAGE LIMIT; no further model requests permitted');
  }
  return item.mission;
}
function alive(pid) {try {process.kill(pid, 0); return true;} catch (error) {if (error.code === 'ESRCH') return false; throw error;}}
function processObservation(pid) {
  return run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', `$items=Get-CimInstance Win32_Process; $owned=[System.Collections.Generic.HashSet[int]]::new(); [void]$owned.Add(${pid}); for($i=0;$i -lt 8;$i++){foreach($item in $items){if($owned.Contains([int]$item.ParentProcessId)){[void]$owned.Add([int]$item.ProcessId)}}}; @($items | Where-Object {$owned.Contains([int]$_.ProcessId)} | Select-Object ProcessId,ParentProcessId,Name,CreationDate)|ConvertTo-Json -Compress`], root);
}
async function worktreeEvidence(folder) {
  const source = await fs.readFile(path.join(folder, 'src/intervals.py'));
  let tests;
  try {tests = await fs.readFile(path.join(folder, 'tests/test_additional_intervals.py'));} catch (error) {if (error.code !== 'ENOENT') throw error;}
  return {folder, sourceSha256: hash(source), additionalTestSha256: tests ? hash(tests) : null, additionalTestContent: tests?.toString('utf8'), status: ok(run('git', ['status', '--porcelain'], folder)), patch: ok(run('git', ['diff', '--', 'src', 'tests'], folder)), head: ok(run('git', ['rev-parse', 'HEAD'], folder))};
}
async function liveWorker(item, requireChanges) {
  return eventually(async () => {
    const mission = await snapshot(item);
    if (!mission) return false;
    if (mission.terminal) throw new Error(`Mission ended before owned interruption: ${JSON.stringify(mission)}`);
    const session = item.detail.sessions.find(value => value.role === 'worker' && value.status === 'running' && value.outputMeta?.processId);
    if (!session) return false;
    const attempt = item.detail.attempts.find(value => value.id === session.attemptId);
    if (!attempt?.worktreePath || !alive(session.outputMeta.processId)) return false;
    if (requireChanges) {
      try {if (!(await fs.readFile(path.join(attempt.worktreePath, 'tests/test_additional_intervals.py'), 'utf8')).includes('def test')) return false;} catch (error) {if (error.code === 'ENOENT') return false; throw error;}
    }
    return {at: new Date().toISOString(), session, attempt, claim: item.scheduler, os: processObservation(session.outputMeta.processId), worktree: await worktreeEvidence(attempt.worktreePath)};
  }, 'actual owned Worker with genuine test changes', 240000, 100);
}
async function semanticAcceptance() {
  const item = result.scenarios.Semantic;
  if (item.qualified) return;
  await submit(item, true);
  let priorState = '';
  await eventually(async () => {
    const mission = await snapshot(item);
    if (!mission) return false;
    const state = `${mission.status}-${mission.plan_revision}-${mission.sessions.map(value => value.status).join(',')}`;
    if (state !== priorState) {console.log('SEMANTIC ' + state); priorState = state; await persist();}
    if (item.detail.attempts.some(value => value.status === 'failed') && !item.initialFailureCapture) {await shot('semantic-original-failure'); item.initialFailureCapture = true;}
    if (mission.plan_revision > 1 && !item.replanCapture) {await shot('semantic-real-replan'); item.replanCapture = true;}
    if (item.detail.sessions.some(value => value.role === 'worker' && value.status === 'running') && !item.workerCapture) {await shot('semantic-real-worker'); item.workerCapture = true;}
    return mission.terminal;
  }, 'genuine semantic repair and review', 1200000, 600);
  await persist();
  assert.equal(item.mission.status, 'completed', 'Mission must actually complete before acceptance');
  assert.ok(item.detail.attempts.some(value => value.status === 'failed'), 'Original behavioral failure must be retained');
  assert.ok(item.scheduler.revisions.some(value => value.source === 'replan' && value.plannerSessionId), 'Actual Planner revision required');
  const workers = item.detail.sessions.filter(value => value.role === 'worker');
  assert.ok(workers.length && workers.every(value => value.outputMeta?.processTreeSupervised));
  const reviewed = item.detail.attempts.filter(value => value.role === 'worker' && value.status === 'approved');
  assert.ok(reviewed.length && reviewed.every(value => value.review?.decision === 'approve' && value.evidence?.diffPatch?.trim()));
  item.patch = ok(run('git', ['diff', item.initialHead, item.mission.integration_sha, '--', 'src', 'tests'], item.root));
  assert.ok(item.patch.includes('src/intervals.py') && item.patch.includes('tests/'));
  assert.equal(ok(run('git', ['rev-parse', `${item.mission.integration_sha}:tests/original_failure.json`], item.root)), item.baselineBlob, 'Original failure evidence is immutable');
  const checkout = path.join(result.fixture, 'verify-semantic-' + Date.now());
  ok(run('git', ['clone', '-q', '--no-hardlinks', item.root, checkout], result.fixture));
  ok(run('git', ['checkout', '-q', item.mission.integration_sha], checkout));
  item.independent = {checkout, head: ok(run('git', ['rev-parse', 'HEAD'], checkout)), tests: run('python', ['-B', '-m', 'unittest', 'discover', '-s', 'tests', '-v'], checkout)};
  assert.equal(item.independent.tests.exit, 0, JSON.stringify(item.independent.tests));
  const oracle = `from itertools import product
from src.intervals import merge_intervals
pairs=[(a,b) for a in range(-2,4) for b in range(a,4)]
count=0
for length in range(4):
    for values in product(pairs,repeat=length):
        original=list(values)
        actual=merge_intervals(original)
        assert original==list(values)
        assert all(a<=b for a,b in actual)
        assert all(actual[i][1]<actual[i+1][0] for i in range(len(actual)-1))
        expected={half for a,b in values for half in range(2*a,2*b+1)}
        observed={half for a,b in actual for half in range(2*a,2*b+1)}
        assert observed==expected,(values,actual)
        assert merge_intervals(actual)==actual
        count+=1
print('Independent closed-interval coverage, ordering, idempotence and input preservation:',count)
`;
  item.independent.oracle = run('python', ['-B', '-c', oracle], checkout);
  assert.equal(item.independent.oracle.exit, 0, JSON.stringify(item.independent.oracle));
  item.roots = await get(item, 'files/roots');
  const integration = item.roots.roots.find(value => value.kind === 'integration_snapshot');
  assert.ok(integration?.read_only);
  item.snapshotFile = await get(item, `files/content?root_id=${integration.id}&path=src/intervals.py`);
  assert.equal(item.snapshotFile.content, await fs.readFile(path.join(checkout, 'src/intervals.py'), 'utf8'));
  await select(item);
  await shot('semantic-completed-conversation');
  await view('Progress');
  await shot('semantic-completed-progress');
  await view('Explorer');
  await page.getByLabel('Filesystem root', {exact: true}).selectOption(integration.id);
  const tree = page.locator('.files-explorer:visible').first().getByRole('tree');
  const directory = tree.getByRole('treeitem').filter({has: page.locator('.files-tree-name', {hasText: /^src$/})}).first();
  if ((await directory.getAttribute('aria-expanded')) !== 'true') await directory.click();
  await tree.getByRole('treeitem').filter({has: page.locator('.files-tree-name', {hasText: /^intervals\.py$/})}).first().click();
  await page.locator('.cm-content').filter({hasText: 'merge_intervals'}).first().waitFor();
  await shot('semantic-integrated-readonly-editor');
  item.mainHead = ok(run('git', ['rev-parse', 'HEAD'], item.root));
  assert.equal(item.mainHead, item.initialHead);
  assert.equal(ok(run('git', ['status', '--porcelain'], item.root)), '');
  assert.equal(item.scheduler.activeClaims, 0);
  item.qualified = new Date().toISOString();
  await persist();
}
async function interruptionAcceptance(restart) {
  const item = result.scenarios[restart ? 'Restart' : 'Cancel'];
  if (item.qualified) return;
  const control = result.scenarios.Control;
  if (!restart) {await submit(control, false, true); item.controlBefore = await liveWorker(control, false);}
  await submit(item);
  item.live = await liveWorker(item, true);
  const pid = item.live.session.outputMeta.processId;
  assert.ok(item.live.session.outputMeta.processTreeSupervised);
  assert.equal(item.live.claim.activeClaims, 1);
  await shot(restart ? 'restart-before-live-interruption' : 'cancel-before-live-interruption');
  item.interruptionAt = new Date().toISOString();
  if (restart) {
    item.ownedServerPid = child.pid;
    await stopServer('controlled-live-worker-server-termination');
  } else {
    await mutate(`/api/projects/${item.project}/runs/${item.mission.run_id}/cancel`, {}, 204);
  }
  await eventually(() => !alive(pid), 'owned native Worker actually terminated', 45000);
  const observed = JSON.parse(item.live.os.stdout || '[]');
  const ownedProcesses = Array.isArray(observed) ? observed : [observed];
  item.descendantsAfter = ownedProcesses.map(value => ({pid: value.ProcessId, name: value.Name, alive: alive(value.ProcessId)}));
  assert.ok(item.descendantsAfter.every(value => !value.alive), 'Observed owned Worker descendants must terminate');
  item.terminationObservedAt = new Date().toISOString();
  item.processAfter = processObservation(pid);
  item.worktreeAfter = await worktreeEvidence(item.live.attempt.worktreePath);
  assert.ok(item.worktreeAfter.additionalTestContent?.includes('def test'), 'Meaningful unsaved Worker artifact remains recoverable');
  if (restart) {
    await startServer();
    csrf = undefined;
    await auth();
    await register();
    await page.goto(base + '/workspace.html');
    await page.getByLabel('Open registered project').waitFor();
    item.worktreeAfterRestart = await worktreeEvidence(item.live.attempt.worktreePath);
    assert.equal(item.worktreeAfterRestart.additionalTestSha256, item.worktreeAfter.additionalTestSha256);
    assert.equal(item.worktreeAfterRestart.sourceSha256, item.worktreeAfter.sourceSha256);
  }
  await eventually(async () => {
    const mission = await snapshot(item);
    return mission && item.scheduler.activeClaims === 0 && !item.scheduler.ownerActive && !item.detail.sessions.some(value => value.status === 'running');
  }, 'durable process ownership and capacity reconciliation');
  assert.ok(!item.detail.attempts.some(value => value.status === 'approved'));
  assert.ok(!item.detail.tasks.some(value => value.state === 'completed'));
  assert.equal(item.scheduler.integrations.filter(value => value.status === 'published').length, 0);
  assert.equal(item.mission.integration_sha, null);
  if (restart) assert.ok(item.detail.attempts.some(value => value.status === 'interrupted'));
  else assert.equal(item.mission.status, 'cancelled');
  const count = item.detail.sessions.length;
  await delay(1500);
  await snapshot(item);
  assert.equal(item.detail.sessions.length, count, 'No blind Worker replay after interruption');
  if (!restart) {
    await snapshot(control);
    item.controlAfter = {status: control.mission.status, sessions: control.detail.sessions, processStillAlive: alive(item.controlBefore.session.outputMeta.processId), claims: control.scheduler.activeClaims};
    assert.ok(item.controlAfter.processStillAlive, 'Other project Worker must be unaffected');
    assert.equal(item.controlAfter.claims, 1);
    await mutate(`/api/projects/${control.project}/runs/${control.mission.run_id}/cancel`, {}, 204);
    await eventually(async () => {await snapshot(control); return control.scheduler.activeClaims === 0 && !alive(item.controlBefore.session.outputMeta.processId);}, 'controlled sentinel cleanup');
  }
  await select(item);
  await view('Progress');
  await shot(restart ? 'restart-reconciled-progress' : 'cancel-reconciled-progress');
  await page.getByRole('status').filter({hasText: /^Workspace saved$/}).waitFor().catch(() => {});
  await page.reload();
  await page.getByLabel('Open registered project').waitFor();
  await select(item);
  await view('Progress');
  await snapshot(item);
  assert.equal(item.detail.sessions.length, count);
  await shot(restart ? 'restart-durable-history' : 'cancel-durable-history');
  await replay(item);
  item.mainHead = ok(run('git', ['rev-parse', 'HEAD'], item.root));
  assert.equal(item.mainHead, item.initialHead);
  assert.equal(ok(run('git', ['status', '--porcelain'], item.root)), '');
  item.qualified = new Date().toISOString();
  await persist();
}
async function test(name, fn) {
  const start = performance.now();
  try {await fn(); result.tests.push({name, status: 'passed', ms: performance.now() - start}); console.log('PASS ' + name);}
  catch (error) {result.tests.push({name, status: 'failed', error: safeError(error), ms: performance.now() - start}); throw error;}
  finally {await persist();}
}
try {
  await startServer();
  browser = await chromium.launch({channel: 'msedge', headless: true});
  context = await browser.newContext({viewport: {width: 1440, height: 1000}, colorScheme: 'dark', reducedMotion: 'reduce'});
  page = await context.newPage();
  page.on('pageerror', error => result.pageErrors.push(String(error)));
  await auth();
  await register();
  await page.goto(base + '/workspace.html');
  await page.getByLabel('Open registered project').waitFor();
  await page.getByLabel('Theme', {exact: true}).selectOption('dark');
  if (mode !== 'interruptions') await test('Semantic failure → actual Planner revision → genuine Worker repair → real review → verified integration', semanticAcceptance);
  if (mode !== 'semantic') {
    await test('A: supported cancellation of an active native Worker preserves artifacts, capacity and another project', () => interruptionAcceptance(false));
    await test('B: owned runtime termination with active native Worker reconciles safely after restart', () => interruptionAcceptance(true));
  }
  await page.getByLabel('Theme', {exact: true}).selectOption('light');
  await shot('recovered-light-theme');
  const legacy = await context.newPage();
  await legacy.goto(base + '/');
  await legacy.getByRole('button', {name: 'Agent Graph', exact: true}).waitFor();
  await legacy.close();
  assert.equal(result.pageErrors.length, 0, JSON.stringify(result.pageErrors));
  result.status = 'PASS';
} catch (error) {
  result.status = result.providerRefused ? 'PROVIDER_BLOCKED' : 'FAIL';
  result.failure = safeError(error);
  console.error(safeError(error));
  process.exitCode = 1;
  if (page) await shot('failure-' + Date.now()).catch(() => {});
} finally {
  result.finished = new Date().toISOString();
  await persist();
  await browser?.close();
  await stopServer('harness-cleanup');
}
