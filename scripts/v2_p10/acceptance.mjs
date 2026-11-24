import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const native = process.argv.includes("--native");
const output = process.env.ASF_P10_OUTPUT ?? path.join(root, `.factory/v2/p10/${native ? "native" : "controlled"}`);
const binary = process.env.ASF_P10_BINARY;
assert.ok(binary, "ASF_P10_BINARY must select a copied, embedded P10 executable");
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium, request } = require("playwright");
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const cli = process.env.ASF_P10_CODEX ?? "codex";
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const file = path.join(output, "results.json");
try { await fs.access(file); throw new Error("Acceptance evidence exists. Preserve it and select a new output directory; do not blindly repeat model work."); } catch (error) { if (error.code !== "ENOENT") throw error; }
if (native) assert.equal(JSON.parse(await fs.readFile(path.join(root, ".factory/v2/p10/preflight/preflight.json"), "utf8")).write, "PASS");
const result = { schema: 1, mode: native ? "real-model" : "controlled-processes", sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).stdout.trim(), binary: path.resolve(binary), binarySha256: hash(await fs.readFile(binary)), scriptSha256: hash(await fs.readFile(fileURLToPath(import.meta.url))), startedAt: new Date().toISOString(), fixture: await fs.mkdtemp(path.join(os.tmpdir(), "asf-p10-")), projects: {}, timeline: [], browserEvents: [], memorySamples: [], browsers: [], screenshots: [], pageErrors: [] };
const persist = () => fs.writeFile(file, JSON.stringify(result, null, 2) + "\n");
async function note(event, data = {}) {
  result.timeline.push({ event, at: new Date().toISOString(), ...data });
  if (child?.pid && child.exitCode===null) {
    const sample=run("powershell.exe",["-NoProfile","-Command",`$p=Get-Process -Id ${Number(child.pid)} -ErrorAction SilentlyContinue; if($p){@{pid=$p.Id;workingSet=$p.WorkingSet64;privateBytes=$p.PrivateMemorySize64}|ConvertTo-Json -Compress}`],result.fixture,10_000);
    try {result.memorySamples.push({event,at:new Date().toISOString(),...JSON.parse(sample.stdout)});}catch{}
  }
  await persist();
}
function run(program, args, cwd, timeout = 90_000) {
  const start = performance.now();
  const out = spawnSync(program, args, { cwd, windowsHide: true, encoding: "utf8", timeout, env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" } });
  return { program, args, cwd, exit: out.status, stdout: out.stdout ?? "", stderr: out.stderr ?? "", error: out.error?.message, ms: performance.now() - start };
}
function checked(record) { assert.equal(record.exit, 0, JSON.stringify(record)); return record.stdout.trim(); }
async function eventually(action, description, timeout = 90_000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) { const value = await action(); if (value) return value; await delay(250); }
  throw new Error(`Timed out: ${description}`);
}
const objective = "In one atomic Worker implementation task, inspect src/chunks.py, fix chunking so a final short chunk is retained, reject nonpositive, boolean and noninteger sizes with ValueError, and add meaningful regression tests. Preserve valid existing behavior. Run python -B -m unittest discover -s tests -v, including the bounded lifecycle validation delay. Do not remove, weaken or skip existing tests. ASF owns commits, independent review and integration. No downloads, publication, policy changes, approvals or work outside the assigned worktree.";
const common = "Respect Rust policy and the existing CLI sandbox. Use individual direct commands, no shell chains or interpreter command bundles. Do not stage or commit; ASF captures evidence. Report exact unannotated command invocations and actual outcomes as the required structured JSON. Do not edit tests/baseline_result.json. Partial work left by an interrupted process remains unapproved and must be inspected and validated.";
function configuration() {
  let text = '[runtime]\nmax_parallel_tasks=1\n[scheduler]\nmax_parallel_tasks=1\nmax_project_tasks=1\nmax_replans=0\nmax_total_tasks=8\nmax_duration_secs=1200\n[[integration_checks]]\nprogram="python"\nargs=["-B","-m","unittest","discover","-s","tests","-v"]\ntimeout_secs=90\n';
  for (const role of ["planner", "worker", "reviewer"]) {
    const args = native ? ["exec", "--ignore-user-config", "--ephemeral", "--disable", "apps", "--sandbox", role === "worker" ? "workspace-write" : "read-only", "-c", "windows.sandbox=mxc", "-c", "allow_login_shell=false", "--color", "never"] : [".factory/controlled.py", role];
    text += `\n[agents.${role}]\nkind=${JSON.stringify(native ? "codex" : "custom")}\ncommand=${JSON.stringify(native ? cli : "python")}\nargs=${JSON.stringify(args)}\nmax_concurrency=1\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE="1"\n[[role_assignments]]\nrole="${role}"\nagent="${role}"\npreferred=true\n[policies.roles.${role}.filesystem]\nread=["**"]\nwrite=${role === "worker" ? '["src/**","tests/**"]' : "[]"}\ndeny_write=["tests/baseline_result.json"]\n[policies.roles.${role}.commands]\nmode="restricted"\nallow=["python","python3","git","rg","Get-Content","Set-Content","Get-ChildItem","Get-Location","Test-Path","powershell","powershell.exe"]\ndeny=["curl","Invoke-WebRequest","cmd","bash","sh"]\n[policies.roles.${role}.network]\nmode="deny"\n`;
  }
  return text;
}
for (const name of ["A", "B"]) {
  const folder = path.join(result.fixture, name);
  await fs.mkdir(path.join(folder, "src"), { recursive: true }); await fs.mkdir(path.join(folder, "tests"));
  const source = "def chunks(items, size):\n    return [items[i:i + size] for i in range(0, len(items) - size + 1, size)]\n";
  const tests = `import time\nimport unittest\nfrom src.chunks import chunks\n\nclass ChunkTests(unittest.TestCase):\n    def test_full_chunks(self): self.assertEqual(chunks([1,2,3,4], 2), [[1,2],[3,4]])\n    def test_remainder(self): self.assertEqual(chunks([1,2,3], 2), [[1,2],[3]])\n    def test_empty(self): self.assertEqual(chunks([], 2), [])\n    def test_lifecycle_validation_delay(self):\n        time.sleep(${native ? 35 : 0})\n        self.assertEqual(chunks([1,2], 1), [[1],[2]])\n`;
  for (const [f, value] of Object.entries({ ".gitignore": ".factory/\n__pycache__/\n*.pyc\n", "src/__init__.py": "", "tests/__init__.py": "", "src/chunks.py": source, "tests/test_chunks.py": tests })) await fs.writeFile(path.join(folder, f), value);
  const baseline = run("python", ["-B", "-m", "unittest", "tests.test_chunks.ChunkTests.test_remainder", "-v"], folder); assert.notEqual(baseline.exit, 0); assert.match(baseline.stderr, /FAIL: test_remainder/);
  await fs.writeFile(path.join(folder, "tests/baseline_result.json"), JSON.stringify({ schema: 1, observedAt: new Date().toISOString(), sourceSha256: hash(source), testsSha256: hash(tests), baseline }, null, 2) + "\n");
  for (const argv of [["init", "-q", "-b", "main"], ["config", "user.name", "ASF P10 Acceptance"], ["config", "user.email", "p10@example.invalid"], ["add", "."], ["commit", "-qm", "disposable failing baseline"]]) checked(run("git", argv, folder));
  checked(run(binary, ["init"], folder)); await fs.writeFile(path.join(folder, ".factory/config.toml"), configuration());
  if (!native) {
    const controlled = path.join(folder, ".factory/controlled.py");
    await fs.writeFile(controlled, `import json,pathlib,sys,time\ntext=sys.stdin.read()\nmode=sys.argv[1]\nif mode=='planner':\n print(json.dumps({'objective':'controlled lifecycle fixture','tasks':[{'id':'edit','title':'Controlled chunk correction','objective':'Controlled source modification and validation','dependencies':[],'acceptanceCriteria':['correct chunks'], 'role':'worker'}]}))\nelif mode=='reviewer':\n print(json.dumps({'decision':'approve','reason':'Controlled fixture decision; not model acceptance','feedback':[]}))\nelse:\n pathlib.Path('src/chunks.py').write_text('def chunks(items, size):\\n    if isinstance(size, bool) or not isinstance(size, int) or size <= 0:\\n        raise ValueError("size must be a positive integer")\\n    return [items[i:i+size] for i in range(0,len(items),size)]\\n')\n pathlib.Path('tests/test_extra.py').write_text('import unittest\\nfrom src.chunks import chunks\\nclass Extra(unittest.TestCase):\\n    def test_larger(self): self.assertEqual(chunks([1],3), [[1]])\\n')\n time.sleep(20 if 'ASF_LOGICAL_RESUME' in text else 60)\n print(json.dumps({'commands':[],'summary':'Controlled lifecycle process fixture','tests':[]}))\n`);
    let config = await fs.readFile(path.join(folder, ".factory/config.toml"), "utf8");
    config = config.replaceAll(JSON.stringify(".factory/controlled.py"), JSON.stringify(controlled.replaceAll("\\", "/"))); await fs.writeFile(path.join(folder, ".factory/config.toml"), config);
  }
  result.projects[name] = { root: folder, baseline, initialHead: checked(run("git", ["rev-parse", "HEAD"], folder)) };
}
let child, inspector, csrf, base, port;
const browsers = [];
const acknowledgements = new Map();
async function api(url, method = "GET", data, headers = {}) {
  const response = await inspector.fetch(base + url, { method, headers: { Origin: base, "X-Factory-CSRF": csrf, ...headers }, ...(data === undefined ? {} : { data }) });
  const raw = await response.text(); const value = raw ? JSON.parse(raw) : null; assert.ok(response.ok(), `${method} ${url}: ${response.status()} ${JSON.stringify(value)}`); return value;
}
const get = (item, suffix) => api(`/api/projects/${item.projectId}/${suffix}`);
async function startServer() {
  if (!port) port = await new Promise((resolve) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); }); });
  base = `http://127.0.0.1:${port}`;
  const log = `server-${Date.now()}.log`; const handle = await fs.open(path.join(output, log), "w");
  child = spawn(binary, ["start", "--no-browser", "--port", String(port), "--registry", path.join(result.fixture, "registry.sqlite3"), "--allow-project-root", result.fixture, "--max-active-tasks", "2"], { cwd: result.projects.A.root, windowsHide: true, stdio: ["ignore", handle.fd, handle.fd], env: { ...process.env, ASF_BROWSER_LEASE_SECONDS: "6", ASF_BROWSER_GRACE_SECONDS: "3" } }); await handle.close();
  await eventually(async () => { try { return (await fetch(base + "/api/health")).ok; } catch { return false; } }, "owned server startup");
  inspector = await request.newContext(); const auth = await inspector.post(base + "/api/auth/session", { headers: { Origin: base }, data: {} }); assert.ok(auth.ok()); csrf = (await auth.json()).csrfToken;
  await note("server.started", { pid: child.pid, log, base });
}
async function stopServer(reason) {
  if (!child || child.exitCode !== null) return;
  const pid = child.pid; const exited = new Promise((resolve) => child.once("exit", resolve)); child.kill(); await exited;
  await note("server.terminated", { pid, reason }); await inspector?.dispose();
}
async function openBrowser(workspaceId) {
  const browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--disable-gpu", "--disable-extensions", "--disable-background-networking"] }); browsers.push(browser);
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce" });
  if (workspaceId) await context.addInitScript((id) => localStorage.setItem("asf.v2.workspace.last", id), workspaceId);
  const page = await context.newPage(); const id = `client-${browsers.length}`;
  const cdp=await context.newCDPSession(page);await cdp.send("Network.enable");
  cdp.on("Network.eventSourceMessageReceived",(event)=>{try{const message=JSON.parse(event.data);if(message.schema_version===1 && result.browserEvents.length<2000)result.browserEvents.push({client:id,id:message.id,conversationId:message.conversation_id,eventType:message.event_type,timestamp:message.timestamp,receivedAt:new Date().toISOString()});}catch{}});
  page.on("pageerror", (e) => result.pageErrors.push({ client: id, error: String(e) }));
  page.on("response", async (response) => { if (new URL(response.url()).pathname.includes("/browser-clients/") && response.ok()) { try { const lease = await response.json(); if (lease.leaseId) { acknowledgements.set(page, lease); if(result.browsers.length<4096)result.browsers.push({ client: id, ...lease, observedAt: new Date().toISOString() }); } } catch {} } });
  await page.goto(base + "/workspace.html"); await page.getByLabel("Open registered project").waitFor(); return { browser, context, page, id };
}
async function select(client, item) { await client.page.getByLabel("Open registered project").selectOption(item.projectId); await settled(client); }
async function view(client, label) { await client.page.getByRole("button", { name: "Views", exact: true }).click(); await client.page.getByRole("menuitem", { name: label, exact: true }).click(); }
async function settled(client) { await eventually(async () => (await client.page.locator(".ws-status").innerText()).includes("Workspace saved"), "durable workspace layout"); }
async function bound(client, item) { return eventually(async () => { const l = acknowledgements.get(client.page); return l?.bindings.some((b) => b.projectId === item.projectId && b.sessionId === item.sessionId) && l; }, "acknowledged browser/session binding"); }
async function shot(client, name) { const filename = `screenshots/${name}.png`; await client.page.screenshot({ path: path.join(output, filename) }); result.screenshots.push({ file: filename, sha256: hash(await fs.readFile(path.join(output, filename))) }); await persist(); }
async function mission(client, item) {
  await select(client, item); await view(client, "Orchestrator");
  const panel = client.page.locator(".chat-panel:visible").last();
  const creating = client.page.waitForResponse((r) => r.request().method() === "POST" && new URL(r.url()).pathname.endsWith("/conversations")); await panel.getByRole("button", { name: "New conversation", exact: true }).click(); item.conversationId = (await (await creating).json()).id;
  await panel.getByRole("button", { name: "Mission form", exact: true }).click(); await panel.getByLabel("Mission objective", { exact: true }).fill(objective); await panel.getByLabel("Acceptance criteria, one per line", { exact: true }).fill("The original remainder regression and existing valid behavior pass. Add invalid-size edge cases. Preserve the bounded validation delay.");
  const submitted = client.page.waitForResponse((r) => r.request().method() === "POST" && new URL(r.url()).pathname.endsWith("/messages")); await panel.getByRole("button", { name: "Plan and start mission", exact: true }).click();
  const response = await submitted; assert.equal(response.status(), 202, await response.text()); item.submission = response.request().postDataJSON(); item.sessionId = item.submission.workspace_session_id; item.command = await response.json();
  const snapshot = await eventually(async () => (await get(item, `conversations/${item.conversationId}/missions`))[0], "real associated workflow"); item.runId = snapshot.run_id;
  await note("mission.accepted", { projectId: item.projectId, sessionId: item.sessionId, runId: item.runId }); await bound(client, item); await settled(client);
}
async function activeWorker(item, after = 0) {
  return eventually(async () => { const detail = await get(item, `runs/${item.runId}`); if (["blocked", "failed", "cancelled"].includes(detail.run.status)) throw new Error(`Workflow stopped: ${detail.run.status}; inspect retained sessions without repeating model requests`); return detail.sessions.find((s) => s.role === "worker" && s.id > after && s.status === "running" && s.outputMeta?.processId); }, "owned coding Worker", 180_000);
}
function processAbsent(pid) { const r = run("powershell.exe", ["-NoProfile", "-Command", `if (Get-Process -Id ${Number(pid)} -ErrorAction SilentlyContinue) { exit 1 }`], result.fixture, 10_000); return r.exit === 0; }
async function suspended(item) { return eventually(async () => { const v = await get(item, `runs/${item.runId}/lifecycle`); if (v.lifecycle?.state === "recovery_required") throw new Error(`Suspension requires inspection: ${v.lifecycle.problem}`); return v.lifecycle?.state === "suspended" && v; }, "confirmed durable suspension", 120_000); }
async function progress(client, item) { await select(client, item); await view(client, "Progress"); await client.page.getByRole("region", { name: `Mission ${item.runId} lifecycle`, exact: true }).waitFor(); }
async function resume(client, item) {
  await progress(client, item); await bound(client, item);
  const requested=performance.now();const response = client.page.waitForResponse((r) => r.request().method() === "POST" && new URL(r.url()).pathname.endsWith(`/runs/${item.runId}/resume`));
  await client.page.getByRole("region", { name: `Mission ${item.runId} lifecycle`, exact: true }).getByRole("button", { name: "Validate and resume", exact: true }).click(); const r = await response; assert.equal(r.status(), 200, await r.text());
  const lease=await bound(client,item);const token=(await client.context.request.post(base+"/api/auth/session",{headers:{Origin:base},data:{}}));assert.ok(token.ok());const tokenBody=await token.json();
  const duplicate=await client.context.request.post(r.url(),{headers:{Origin:base,"X-Factory-CSRF":tokenBody.csrfToken,"X-ASF-Lease":lease.leaseId},data:r.request().postDataJSON()});assert.equal(duplicate.status(),200,await duplicate.text());
  item.resumeCommands ??= []; item.resumeCommands.push(r.request().postDataJSON()); await note("mission.resume_acknowledged", { projectId: item.projectId, requestToAcknowledgementMs:performance.now()-requested,response: await r.json() });
}
async function closeSession(client, item) {
  await select(client, item); await client.page.getByRole("button", { name: "Close session", exact: true }).click(); const dialog = client.page.getByRole("dialog", { name: "Close workspace session", exact: true }); await dialog.waitFor(); await shot(client, `close-${item.projectId}`);
  await dialog.getByRole("button", { name: "Close and suspend missions", exact: true }).click(); await suspended(item); await note("session.closed", { projectId: item.projectId, sessionId: item.sessionId });
}
async function profiles(item) {
  item.profiles = {};
  for (const role of ["planner", "worker", "reviewer"]) {
    const profile = await api(`/api/projects/${item.projectId}/agent-profiles`, "POST", { name: `P10 ${role}`, description: "Isolated P10 acceptance profile", providerAdapterId: role, modelId: native ? "gpt-6.1-sol" : null, reasoningEffort: native ? "low" : null, roles: [role], instructions: `${common} ${role === "planner" ? "For this bounded request produce exactly one implementation task including tests." : ""}`, permissions: {}, maxConcurrency: 1, timeoutSecs: 300, enabled: true, automaticDelegation: true, preferred: true }); item.profiles[role] = { id: profile.id, revision: profile.revision };
  }
}
async function verify(item, name) {
  const detail = await get(item, `runs/${item.runId}`); assert.equal(detail.run.status, "completed"); item.finalDetail = { run: detail.run, tasks: detail.tasks, attempts: detail.attempts, integration: detail.integration };
  const scheduling = await get(item, `runs/${item.runId}/scheduler`); item.integrations = scheduling.integrations; assert.ok(item.integrations.some((i) => i.status === "published")); const sha = item.integrations.filter((i) => i.status === "published").at(-1).candidate_sha; item.integrationCommit = sha;
  item.sessions = detail.sessions.map((s) => ({ id: s.id, agent: s.agent, taskId: s.taskId, attemptId: s.attemptId, role: s.role, status: s.status, exitCode: s.exitCode, outputMeta: s.outputMeta, stdoutSha256: hash(s.stdout ?? ""), stderrSha256: hash(s.stderr ?? ""), nativeHeader: (s.stderr ?? "").split(/\r?\n/).slice(0, 30).filter((l) => /^(OpenAI Codex|model:|approval:|sandbox:|workdir:)/.test(l)) }));
  if (native) assert.ok(item.sessions.some((s) => s.role === "worker" && s.nativeHeader.includes("model: gpt-6.1-sol")));
  item.snapshots = [];
  for (const s of detail.sessions) item.snapshots.push({ sessionId: s.id, snapshot: await get(item, `agent-sessions/${s.id}/execution-snapshot`), observation: await get(item, `agent-sessions/${s.id}/model-observation`) });
  const verification = path.join(result.fixture, `verification-${name}`); checked(run("git", ["clone", "--no-checkout", "--no-hardlinks", "--local", item.root, verification], result.fixture)); checked(run("git", ["remote", "remove", "origin"], verification)); checked(run("git", ["checkout", "--detach", sha], verification));
  item.verification = run("python", ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"], verification); checked(item.verification);
  item.oracle = run("python", ["-B", "-c", "from src.chunks import chunks\nfor n in range(10):\n for size in range(1,12):\n  data=list(range(n)); got=chunks(data,size); assert [v for group in got for v in group]==data; assert all(0<len(group)<=size for group in got)\nfor size in (0,-1,True,False,1.5,'2',None):\n try: chunks([1,2,3],size)\n except ValueError: pass\n else: raise AssertionError(size)\nprint('110 partition cases and 7 invalid-size cases passed')"], verification); checked(item.oracle);
  item.diff = checked(run("git", ["diff", "--stat", item.initialHead, sha], item.root)); assert.match(item.diff, /src\/chunks.py/); await fs.writeFile(path.join(output, `diff-${name}.patch`), checked(run("git", ["diff", "--binary", item.initialHead, sha], item.root)) + "\n"); assert.equal(checked(run("git", ["rev-parse", "HEAD"], item.root)), item.initialHead); assert.equal(checked(run("git", ["remote"], item.root)), "");
  assert.ok(detail.attempts.some((a) => a.review?.decision === "approve")); item.checkpoints = await get(item, `runs/${item.runId}/checkpoints`); item.events = await get(item, `runs/${item.runId}/scheduler/events?after=0`); item.messages = await get(item, `conversations/${item.conversationId}/messages?limit=100`); await note("mission.independently_verified", { projectId: item.projectId, sha, testExit: item.verification.exit, oracleExit: item.oracle.exit });
}
async function inspectIntegratedFile(client,item) {
  await select(client,item);await client.page.getByRole("button",{name:"Code",exact:true}).click();await view(client,"Explorer");
  const roots=(await get(item,"files/roots")).roots;
  const integration=roots.find((r)=>r.kind==="integration_snapshot");assert.ok(integration?.read_only);
  const explorer=client.page.locator(".files-explorer:visible").first();await explorer.getByLabel("Filesystem root",{exact:true}).selectOption(integration.id);
  const tree=explorer.getByRole("tree");const directory=tree.getByRole("treeitem").filter({has:client.page.locator(".files-tree-name",{hasText:/^src$/})}).first();
  if(await directory.getAttribute("aria-expanded")!=="true")await directory.click();
  await tree.getByRole("treeitem").filter({has:client.page.locator(".files-tree-name",{hasText:/^chunks\.py$/})}).first().click();
  const editor=client.page.locator(".files-editor:visible").last();await editor.locator(".cm-content").waitFor();assert.equal(await editor.locator(".cm-content").getAttribute("contenteditable"),"false");assert.match(await editor.locator(".cm-content").innerText(),/def chunks/);
  await client.page.getByRole("button",{name:"Layout actions",exact:true}).click();await client.page.getByRole("menuitem",{name:"Toggle maximization",exact:true}).click();await settled(client);await shot(client,"verified-integration-codemirror");
  item.editorInspection={rootId:integration.id,kind:integration.kind,readOnly:true,sourceText:await editor.locator(".cm-content").innerText()};
}
try {
  await startServer();
  const entries = await api("/api/projects");
  for (const item of Object.values(result.projects)) {
    let project = entries.find((e) => path.resolve(e.canonical_root.replace(/^\\\\\?\\/, "")).toLowerCase() === path.resolve(item.root).toLowerCase());
    if (!project) project = await api("/api/projects/register", "POST", { path: item.root, trust_project: true, display_name: `P10 ${path.basename(item.root)}` }); item.projectId = project.project_id; await api(`/api/projects/${item.projectId}/open`, "POST"); await profiles(item);
  }
  result.presencePerformance=[];
  for(const count of [1,5,20]) {
    const leases=[];
    for(let i=0;i<count;i++){
      const clientInstanceId="bc_"+createHash("sha256").update(`${count}:${i}:${Date.now()}`).digest("hex").slice(0,32);const started=performance.now();const lease=await api("/api/browser-clients/register","POST",{clientInstanceId,bindings:[]});leases.push(lease);result.presencePerformance.push({clients:count,operation:"register",ms:performance.now()-started});
    }
    for(const lease of leases){const started=performance.now();await api(`/api/browser-clients/${lease.leaseId}/heartbeat`,"POST",{clientInstanceId:lease.clientInstanceId,generation:lease.generation,sequence:1,bindings:[]});result.presencePerformance.push({clients:count,operation:"heartbeat",ms:performance.now()-started});await api(`/api/browser-clients/${lease.leaseId}/release`,"POST",{clientInstanceId:lease.clientInstanceId,generation:lease.generation,sequence:1,bindings:[]});}
  }
  await note("presence.performance_measured",{samples:result.presencePerformance.length});
  const a = result.projects.A, b = result.projects.B;
  const first = await openBrowser(); await mission(first, a); result.workspaceId = await first.page.evaluate(() => localStorage.getItem("asf.v2.workspace.last")); const workerA = await activeWorker(a); a.firstWorker = { id: workerA.id, process: workerA.outputMeta }; assert.equal(workerA.outputMeta.processTreeSupervised, true); await shot(first, "worker-active");
  const second = await openBrowser(result.workspaceId); await bound(second, a); await settled(first); await first.browser.close(); await note("browser.first_closed"); await delay(11_000); assert.equal((await get(a, `runs/${a.runId}/lifecycle`)).lifecycle.state, "active"); assert.equal(processAbsent(workerA.outputMeta.processId), false); await note("browser.other_client_preserved_execution");
  await eventually(async () => { const detail = await get(a, `runs/${a.runId}`); const task = detail.tasks.find((t) => t.id === workerA.taskId); return task?.worktreePath && checked(run("git", ["diff", "--stat"], task.worktreePath)).includes("chunks.py") && detail.sessions.some((s) => s.id === workerA.id && s.status === "running"); }, "model-originated partial source change", 120_000);
  await second.browser.close(); await note("browser.final_closed"); a.firstSuspension = await suspended(a); assert.ok(processAbsent(workerA.outputMeta.processId)); a.firstCheckpoint = await get(a, `runs/${a.runId}/checkpoints`); assert.ok(a.firstCheckpoint[0].worktrees.length); await note("mission.absence_suspended", { generation: a.firstSuspension.lifecycle.generation, checkpoint: a.firstCheckpoint[0].id });
  const returned = await openBrowser(result.workspaceId); await progress(returned, a); await bound(returned, a); assert.equal((await get(a, `runs/${a.runId}/lifecycle`)).lifecycle.state, "suspended"); await shot(returned, "suspended-after-return");
  await mission(returned, b); const workerB = await activeWorker(b); b.firstWorker = { id: workerB.id, process: workerB.outputMeta }; await resume(returned, a); const workerA2 = await activeWorker(a, workerA.id); a.secondWorker = { id: workerA2.id, process: workerA2.outputMeta }; await closeSession(returned, a); assert.ok(processAbsent(workerA2.outputMeta.processId)); assert.equal((await get(b, `runs/${b.runId}/lifecycle`)).lifecycle.state, "active"); assert.equal(processAbsent(workerB.outputMeta.processId),false); await note("session.other_project_preserved_execution"); await shot(returned, "explicit-session-suspended"); await settled(returned);
  await eventually(async () => { const detail = await get(b, `runs/${b.runId}`); const t = detail.tasks.find((t) => t.id === workerB.taskId); return t?.worktreePath && checked(run("git", ["diff", "--stat"], t.worktreePath)).includes("chunks.py") && detail.sessions.some((s) => s.id === workerB.id && s.status === "running"); }, "second model-originated partial source change", 120_000);
  await note("runtime.crash_during_owned_worker", { projectId: b.projectId, sessionId: workerB.id, process: workerB.outputMeta }); await stopServer("controlled abrupt runtime interruption, not graceful suspension"); await returned.browser.close(); assert.ok(processAbsent(workerB.outputMeta.processId)); await startServer(); await api(`/api/projects/${b.projectId}/open`, "POST"); b.restartState = await get(b, `runs/${b.runId}/lifecycle`); assert.equal(b.restartState.lifecycle.state, "recovery_required"); assert.equal(b.restartState.lifecycle.reason, "runtime_interruption"); assert.equal((await get(a, `runs/${a.runId}/lifecycle`)).lifecycle.state, "suspended");
  const recovery = await openBrowser(result.workspaceId); await progress(recovery, b); await bound(recovery, b); await shot(recovery, "runtime-recovery-required"); await resume(recovery, b); b.resumedWorker = { id: (await activeWorker(b, workerB.id)).id };
  await select(recovery, a); await recovery.page.getByRole("button", { name: "Reopen session", exact: true }).click(); await eventually(async () => !(await get(a, `workspace-sessions/${a.sessionId}/lifecycle`)).closed, "explicit session reopen"); await resume(recovery, a); a.resumedWorker = { id: (await activeWorker(a, workerA2.id)).id }; await shot(recovery, "resumed-remaining-work");
  for (const item of [a, b]) await eventually(async () => { const detail = await get(item, `runs/${item.runId}`); if (["failed", "blocked", "cancelled"].includes(detail.run.status)) throw new Error(`Real mission ${item.projectId} stopped: ${detail.run.status}; inspect preserved failure without automatic repetition`); return detail.run.status === "completed"; }, "verified remaining workflow completion", 900_000);
  await verify(a, "A"); await verify(b, "B"); await progress(recovery, a); await recovery.page.getByLabel("Theme", { exact: true }).selectOption("dark"); await shot(recovery, "completed-dark"); await recovery.page.getByLabel("Theme", { exact: true }).selectOption("light"); await shot(recovery, "completed-light"); await settled(recovery); await recovery.page.reload(); await progress(recovery, a); assert.match(await recovery.page.locator(".chat-mission:visible").innerText(), /completed/); await shot(recovery, "restored-completion");
  await view(recovery,"Orchestrator");await eventually(async()=> (await recovery.page.locator(".chat-message:visible").count())>=a.messages.length,"durable lifecycle messages in conversation");
  const messageIds=await recovery.page.locator(".chat-message:visible").evaluateAll((rows)=>rows.map((r)=>r.getAttribute("data-message-id")));assert.equal(new Set(messageIds).size,messageIds.length);await shot(recovery,"completed-conversation");
  await inspectIntegratedFile(recovery,a);
  const v1=await recovery.context.newPage();await v1.goto(`${base}/?project=${a.projectId}`);await v1.getByRole("table").waitFor();await v1.close();
  assert.deepEqual(result.pageErrors, []); result.agentInvocationCount = Object.values(result.projects).reduce((n, p) => n + p.sessions.length, 0); result.modelInvocationCount = native ? result.agentInvocationCount : 0; result.status = "PASS"; await note("acceptance.completed", { modelInvocationCount: native ? result.modelInvocationCount : 0 });
} catch (error) { result.status = "FAILED"; result.error = String(error?.stack ?? error); for(const [i,browser] of browsers.entries())for(const context of browser.contexts())for(const page of context.pages()){try{const filename=`screenshots/failure-${i}.png`;await page.screenshot({path:path.join(output,filename),timeout:5000});result.screenshots.push({file:filename,sha256:hash(await fs.readFile(path.join(output,filename)))});}catch{}} await note("acceptance.failed", { error: result.error }); throw error; }
finally { for (const browser of browsers) await browser.close().catch(() => {}); await stopServer("owned acceptance cleanup").catch(() => {}); await persist(); }
