import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import net from "node:net";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const retryModel = process.argv.includes("--retry-model");
const inspectModel = process.argv.includes("--inspect-model");
const model = process.argv.includes("--model") || retryModel || inspectModel;
const retainedReplay = process.argv.includes("--retained-replay");
const retained = process.argv.includes("--retained") || retainedReplay;
const performanceOnly = process.argv.includes("--performance");
const binary = process.env.ASF_P12_BINARY;
assert.ok(binary, "Select a copied embedded executable using ASF_P12_BINARY");
const output = process.env.ASF_P12_OUTPUT ?? path.join(root, `.factory/v2/p12/${performanceOnly ? "performance" : retained ? "retained" : model ? "model" : "browser"}`);
await fs.mkdir(path.join(output, "screenshots"), { recursive: true });
const recordPath = path.join(output, "results.json");
try { await fs.access(recordPath); throw Error("Evidence already exists. Select a new output directory."); } catch (e) { if (e.code !== "ENOENT") throw e; }
const require = createRequire(path.join(root, "apps/dashboard/package.json"));
const { chromium, request } = require("playwright");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const result = { schema: 1, mode: model ? "genuine-model-review" : "real-Git-production-browser-no-model", sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).stdout.trim(), binarySha256: hash(await fs.readFile(binary)), scriptSha256: hash(await fs.readFile(fileURLToPath(import.meta.url))), startedAt: new Date().toISOString(), fixture: await fs.mkdtemp(path.join(os.tmpdir(), "asf-p12-")), projects: {}, tests: [], screenshots: [], timeline: [], pageErrors: [], remoteWrites: 0, publicationRequests: [] };
let child, inspector, csrf, base, browser, context, page;
const barriers=[];
const persist = () => fs.writeFile(recordPath, JSON.stringify(result, null, 2) + "\n");
function run(program, args, cwd, timeout = 120000) {
  const start = performance.now();
  const r = spawnSync(program, args, { cwd, timeout, encoding: "utf8", windowsHide: true, env: { ...process.env, PYTHONDONTWRITEBYTECODE: "1" } });
  return { program, args, cwd, exit: r.status, stdout: r.stdout ?? "", stderr: r.stderr ?? "", error: r.error?.message, ms: performance.now() - start };
}
function checked(r) { assert.equal(r.exit, 0, JSON.stringify(r)); return r.stdout.trim(); }
async function until(action, label, timeout = 90000) { const end = Date.now() + timeout; while (Date.now() < end) { const value = await action(); if (value) return value; await delay(400); } throw Error(`Timed out: ${label}`); }
function execution(d) { return {...d,sessions:d.sessions.map(({stdout,stderr,...s})=>({...s,stdoutSha256:hash(stdout??""),stderrSha256:hash(stderr??""),providerDiagnostic:((stdout??"")+"\n"+(stderr??"")).split(/\r?\n/).filter(line=>/^(?:error:|authentication failed|not logged in|permission denied|quota exceeded|usage limit|you.?ve hit your usage limit)/i.test(line)).slice(0,10).map(line=>line.slice(0,400)),nativeHeader:(stderr??"").split(/\r?\n/).filter(line=>/^(OpenAI Codex|workdir:|model:|approval:|sandbox:|tokens used)/.test(line)),structuredOutput:(()=>{try{return JSON.parse(stdout??"");}catch{return null;}})()}))}; }
async function note(event, data = {}) { result.timeline.push({ event, at: new Date().toISOString(), ...data }); await persist(); }
async function test(name, action) { const start = performance.now(); await action(); result.tests.push({ name, status: "PASS", ms: performance.now() - start }); await persist(); }
async function api(url, method = "GET", data, busyRetries = 3) {
  const started = performance.now();
  const r = await inspector.fetch(base + url, { method, headers: { Origin: base, "X-Factory-CSRF": csrf }, ...(data === undefined ? {} : { data }), timeout: 240000 });
  const text = await r.text(); const value = text ? JSON.parse(text) : null;
  result.apiTimings ??= [];
  assert.ok(result.apiTimings.length < 2000, "Bounded acceptance request log");
  result.apiTimings.push({ method, path: url, status: r.status(), durationMs: performance.now() - started });
  if(method==="GET" && r.status()===409 && value?.code==="git_busy" && busyRetries>0){await delay(400);return api(url,method,data,busyRetries-1);}
  assert.ok(r.ok(), `${method} ${url}: ${r.status()} ${text}`); return value;
}
const get = (p, url) => api(`/api/projects/${p.id}/${url}`);
const mutate = (p, url, data) => api(`/api/projects/${p.id}/${url}`, "POST", data);
const sc = (p, url, data) => data === undefined ? get(p, `source-control/${url}`) : mutate(p, `source-control/${url}`, data);
const cli = process.env.ASF_P12_CODEX ?? "codex";
function configuration() {
  let text = '[runtime]\nmax_parallel_tasks=2\n[scheduler]\nmax_parallel_tasks=2\nmax_project_tasks=2\nmax_replans=1\nmax_total_tasks=8\nmax_duration_secs=1200\n[[integration_checks]]\nprogram="python"\nargs=["-B","-m","unittest","discover","-s","tests","-v"]\ntimeout_secs=90\n';
  for (const role of ["planner", "worker", "reviewer"]) {
    const args = ["exec", "--ignore-user-config", "--ephemeral", "--disable", "apps", "--sandbox", role === "worker" ? "workspace-write" : "read-only", "-c", "windows.sandbox=mxc", "-c", "allow_login_shell=false", "--color", "never"];
    text += `\n[agents.${role}]\nkind="codex"\ncommand=${JSON.stringify(cli)}\nargs=${JSON.stringify(args)}\nmax_concurrency=1\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE="1"\n[[role_assignments]]\nrole="${role}"\nagent="${role}"\npreferred=true\n[policies.roles.${role}.filesystem]\nread=["**"]\nwrite=${role === "worker" ? '["src/**","tests/**"]' : '[]'}\n[policies.roles.${role}.commands]\nmode="restricted"\nallow=["python","python3","git","rg","Get-Content","Get-ChildItem","Get-Location","Test-Path","powershell","powershell.exe"]\ndeny=["curl","Invoke-WebRequest","cmd","bash","sh"]\n[policies.roles.${role}.network]\nmode="deny"\n`;
  }
  return text;
}
async function fixtures() {
  if(retainedReplay){
    const sourcePath=process.env.ASF_P12_RETAINED_SOURCE ?? path.join(root,".factory/v2/p12/retained-2/results.json");
    const bytes=await fs.readFile(sourcePath);const prior=JSON.parse(bytes);assert.ok(prior.projects.Parallel.execution&&prior.projects.Conflict.execution);result.fixture=prior.fixture;result.projects=prior.projects;result.mode="model-free-replay-of-completed-controlled-scheduler-UI-evidence";result.modelInvocations=0;result.originalRetainedEvidence={path:sourcePath,sha256:hash(bytes),binarySha256:prior.binarySha256};return;
  }
  if(inspectModel){
    const sourcePath=process.env.ASF_P12_MODEL_SOURCE ?? path.join(root,".factory/v2/p12/model-retry-4/results.json");
    const bytes=await fs.readFile(sourcePath);const prior=JSON.parse(bytes);assert.equal(prior.status,"PASS");
    result.fixture=prior.fixture;result.projects=prior.projects;result.cli=prior.cli;result.mode="model-free-continuation-of-recorded-genuine-review";result.modelInvocations=0;result.originalModelEvidence={path:sourcePath,sha256:hash(bytes),binarySha256:prior.binarySha256};return;
  }
  if(retryModel){
    const sourcePath=process.env.ASF_P12_MODEL_SOURCE ?? path.join(root,".factory/v2/p12/model-1/results.json");
    const bytes=await fs.readFile(sourcePath);const prior=JSON.parse(bytes);
    assert.equal(prior.status,"FAILED");assert.ok(prior.projects.A.failedDetail.attempts.some(a=>(a.error??"").includes("commands outside the command policy")));
    assert.ok(!prior.projects.A.failedDetail.sessions.some(s=>s.providerDiagnostic?.some(line=>/usage.limit|quota/i.test(line))));
    result.fixture=prior.fixture;result.projects=prior.projects;result.cli=prior.cli;result.originalModelEvidence={path:sourcePath,sha256:hash(bytes),reason:"Explicit bounded retry after a correctly rejected command report; same policy and preserved worktree"};
    result.projects.A.priorWorker=result.projects.A.worker;delete result.projects.A.worker;return;
  }
  result.tools={git:checked(run("git",["--version"],root)),gh:checked(run("gh",["--version"],root)).split(/\r?\n/)[0]};
  if (model) {
    result.cli = { version: checked(run(cli, ["--version"], root)), loginStatusExit: run(cli, ["login", "status"], root).exit, selectedModel: "gpt-6.1-sol", availability: "Not yet verified by this acceptance" };
    assert.equal(result.cli.loginStatusExit, 0, "CLI authentication unavailable; no model request sent");
  }
  for (const name of ["A", "B"]) {
    const folder = path.join(result.fixture, name); await fs.mkdir(path.join(folder, "src"), { recursive: true }); await fs.mkdir(path.join(folder, "tests"));
    const source = "def merge_ranges(ranges):\n    result = []\n    for start, end in sorted(ranges):\n        if result and start <= result[-1][1]:\n            result[-1] = (result[-1][0], end)\n        else:\n            result.append((start, end))\n    return result\n";
    const tests = "import unittest\nfrom src.ranges import merge_ranges\nclass RangeTests(unittest.TestCase):\n    def test_disjoint(self):self.assertEqual(merge_ranges([(6,8),(1,3)]),[(1,3),(6,8)])\n    def test_touching(self):self.assertEqual(merge_ranges([(1,3),(3,5)]),[(1,5)])\n    def test_nested(self):self.assertEqual(merge_ranges([(1,10),(2,3),(4,6)]),[(1,10)])\n    def test_empty(self):self.assertEqual(merge_ranges([]),[])\n";
    const files = { ".gitignore": ".factory/\n__pycache__/\n*.pyc\n", "src/__init__.py": "", "tests/__init__.py": "", "src/ranges.py": source, "tests/test_ranges.py": tests, "src/manual.py": Array.from({ length: 35 }, (_, n) => `VALUE_${n + 1} = ${n + 1}\n`).join(""), "README.md": `Disposable ${name} Source Control acceptance repository. No publication remote.\n` };
    for (const [f, text] of Object.entries(files)) await fs.writeFile(path.join(folder, f), text);
    for (const args of [["init", "-q", "-b", "main"], ["config", "user.name", "ASF P12 Acceptance"], ["config", "user.email", "p12@example.invalid"], ["config", "core.autocrlf", "false"], ["config", "commit.gpgsign", "false"], ["add", "."], ["commit", "-qm", "disposable source control baseline"]]) checked(run("git", args, folder));
    checked(run(binary, ["init"], folder)); await fs.writeFile(path.join(folder, ".factory/config.toml"), configuration());
    result.projects[name] = { root: folder, initialHead: checked(run("git", ["rev-parse", "HEAD"], folder)) };
  }
  if (retained) for (const [name,label] of [["Parallel","second-parallel"],["Conflict","second-conflict"]]) {
    const record = await fs.readFile(path.join(root,`.factory/private-evidence/p08/${label}/execution.json`));
    const source = JSON.parse(record);
    const folder = path.normalize(source.repository.path);
    try {
      await fs.access(path.join(folder,".factory/db.sqlite3"));
      result.projects[name] = {root:folder, initialHead:source.repository.initial_head, sourceEvidenceSha256:hash(record), sourceIntegrationRecords:source.integration_records, priorOverlapMs:source.overlap_ms, reusedGenuineExecution:true};
    } catch(error) {
      if(error.code!=="ENOENT") throw error;
      const p=await controlledFixture(name);p.historicalSource={path:source.repository.path,sha256:hash(record),databaseAvailable:false,originalOverlapMs:source.overlap_ms};result.projects[name]=p;result.mode="original-P08-records-and-current-controlled-scheduler-UI-qualification";
    }
  }
}
async function controlledFixture(name){
  const folder=path.join(result.fixture,name);await fs.mkdir(path.join(folder,"tests"),{recursive:true});
  for(const [file,text] of Object.entries({".gitignore":".factory/\n__pycache__/\n","base.py":"VALUE = 0\n","shared.py":"FLAGS = {'base': True}\n","tests/test_baseline.py":"import unittest\nfrom base import VALUE\nfrom shared import FLAGS\nclass Baseline(unittest.TestCase):\n def test_base(self): self.assertEqual(VALUE,0); self.assertTrue(FLAGS['base'])\n"}))await fs.writeFile(path.join(folder,file),text);
  for(const args of [["init","-q","-b","main"],["config","user.name","P12 Controlled Scheduler"],["config","user.email","fixture@example.invalid"],["config","core.autocrlf","false"],["config","commit.gpgsign","false"],["add","."],["commit","-qm","controlled scheduler baseline"]])checked(run("git",args,folder));
  checked(run(binary,["init"],folder));
  const source=await fs.readFile(path.join(root,"crates/factory-core/tests/dynamic_scheduler.rs"),"utf8");
  let script=source.match(/const AGENT: &str = r#"([\s\S]*?)"#;/)[1];
  const target="    target='shared.py' if name in ('conflictA','conflictB') else 'part_'+name+'.py'";
  assert.ok(script.includes(target));script=script.replace(target,"    if name in ('conflictA','conflictB','repair'):\n        flags={'base':True}\n        if name in ('conflictA','repair'):flags['A']=True\n        if name in ('conflictB','repair'):flags['B']=True\n        pathlib.Path('shared.py').write_text('FLAGS = '+repr(flags)+'\\n')\n        print(json.dumps({'commands':[],'summary':'controlled conflict or repair source change','tests':[]}))\n        sys.exit(0)\n    target='part_'+name+'.py'");
  await fs.writeFile(path.join(folder,".factory/controlled.py"),script);
  let config='[runtime]\nmax_parallel_tasks=2\n[scheduler]\nmax_parallel_tasks=2\nmax_project_tasks=2\nmax_replans=1\nmax_total_tasks=8\nmax_duration_secs=180\n[[integration_checks]]\nprogram="python"\nargs=["-B","-m","unittest","discover","-s","tests","-v"]\ntimeout_secs=30\n';
  for(const [role,mode] of [["planner","planner"],["worker","worker"],["reviewer","review"]])config+=`\n[agents.${role}]\ncommand="python"\nargs=${JSON.stringify([path.join(folder,".factory/controlled.py"),mode])}\nprompt_transport="stdin"\nmax_concurrency=2\n[agents.${role}.env]\nPYTHONDONTWRITEBYTECODE="1"\n[[role_assignments]]\nrole="${role}"\nagent="${role}"\npreferred=true\n`;
  await fs.writeFile(path.join(folder,".factory/config.toml"),config);
  let ready=false;const waiting=[];const server=net.createServer(socket=>{socket.on("error",()=>{});if(ready)socket.end("G");else{waiting.push(socket);if(waiting.length===2){ready=true;waiting.splice(0).forEach(s=>s.end("G"));}}});
  await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));barriers.push({server,waiting});await fs.writeFile(path.join(folder,".factory/stress-barrier-address"),String(server.address().port));
  return {root:folder,initialHead:checked(run("git",["rev-parse","HEAD"],folder)),controlled:true,modelInvocations:0};
}
async function start() {
  const port = await new Promise(resolve => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const p = s.address().port; s.close(() => resolve(p)); }); });
  base = `http://127.0.0.1:${port}`;
  const h = await fs.open(path.join(output, "server.log"), "w");
  child = spawn(binary, ["start", "--no-browser", "--port", String(port), "--registry", path.join(result.fixture, "registry.sqlite3"), "--allow-project-root", result.fixture, ...Object.values(result.projects).filter(p=>p.reusedGenuineExecution).flatMap(p=>["--allow-project-root",p.root]), "--max-active-tasks", "2"], { cwd: result.projects.A.root, windowsHide: true, stdio: ["ignore", h.fd, h.fd] }); await h.close();
  await until(async () => { try { return (await fetch(base + "/api/health")).ok; } catch { return false; } }, "owned test server");
  inspector = await request.newContext(); const auth = await inspector.post(base + "/api/auth/session", { headers: { Origin: base }, data: {} }); assert.ok(auth.ok()); csrf = (await auth.json()).csrfToken;
  await note("server.started", { pid: child.pid });
  const entries = await api("/api/projects");
  for (const p of Object.values(result.projects)) {
    let entry = entries.find(e => path.resolve(e.canonical_root.replace(/^\\\\\?\\/, "")).toLowerCase() === p.root.toLowerCase());
    if (!entry) entry = await api("/api/projects/register", "POST", { path: p.root, trust_project: true, display_name: `P12 ${path.basename(p.root)}` });
    p.id = entry.project_id; await mutate(p, "open"); p.roots = await get(p, "files/roots"); p.rootId = p.roots.roots.find(r => r.kind === "repository").id;
  }
  browser = await chromium.launch({ channel: "msedge", headless: true, args: ["--disable-gpu", "--disable-extensions", "--disable-background-networking"] });
  context = await browser.newContext({ viewport: { width: 1600, height: 1080 }, reducedMotion: "reduce" });
  if(retryModel||inspectModel){let saved;for(const entry of await api("/api/workspaces")){const doc=(await api(`/api/workspaces/${entry.id}`)).document;if(doc.sessions.some(s=>s.id===result.projects.A.sessionId)){saved=doc;break;}}assert.ok(saved,"Restore actual owning workspace");await context.addInitScript(id=>localStorage.setItem("asf.v2.workspace.last",id),saved.id);result.restoredWorkspace=saved.id;}
  page = await context.newPage(); page.on("pageerror", e => result.pageErrors.push(String(e))); page.on("request",r=>{if(r.method()!=="GET" && /\/source-control\/(delivery|github)/.test(new URL(r.url()).pathname)) result.publicationRequests.push({method:r.method(),path:new URL(r.url()).pathname});});
  await page.goto(base + "/workspace.html"); await page.getByLabel("Open registered project").waitFor(); await page.getByLabel("Theme",{exact:true}).selectOption("dark");
}
async function settled() { let observed="";await until(async () => {observed=await page.locator(".ws-status").innerText();return observed.includes("Workspace saved");}, "durable workspace layout").catch(error=>{throw Error(`${error.message}; observed status: ${observed}`);}); }
async function select(p) { await page.getByLabel("Open registered project").selectOption(p.id); await settled(); await page.getByLabel("Theme",{exact:true}).selectOption("dark"); await settled(); }
async function view(name) { await page.getByRole("button", { name: "Views", exact: true }).click(); await page.getByRole("menuitem", { name, exact: true }).click(); }
async function shot(name) { await settled(); const file = `screenshots/${name}.png`; await page.screenshot({ path: path.join(output, file) }); result.screenshots.push({ file, sha256: hash(await fs.readFile(path.join(output, file))) }); await persist(); }
function panel() { return page.locator(".asf-source:visible").filter({has: page.getByRole("heading", {name: "Source Control", exact: true})}).last(); }
async function refresh() { const waiting = page.waitForResponse(r => r.url().includes("/source-control/worktrees") && r.ok()); await panel().getByRole("button", { name: "Refresh repository", exact: true }).click(); await waiting; }
async function openPath(relative) {
  await view("Explorer"); const tree = page.locator(".files-explorer:visible").last().getByRole("tree");
  for (const [i, part] of relative.split("/").entries()) {
    const row = tree.getByRole("treeitem").filter({ has: page.locator(".files-tree-name", { hasText: new RegExp(`^${part.replaceAll(".", "\\.")}$`) }) }).first();
    await row.waitFor(); if (i === relative.split("/").length - 1 || (await row.getAttribute("aria-expanded")) !== "true") await row.click();
  }
  await page.getByRole("textbox", { name: `Source editor ${relative}`, exact: true }).last().waitFor();
}
async function edit(relative, text, save) {
  const editor = page.getByRole("textbox", { name: `Source editor ${relative}`, exact: true }).last(); await editor.click(); await editor.press("ControlOrMeta+a"); await page.keyboard.insertText(text);
  if (save) { await page.getByRole("toolbar", { name: `File actions ${relative}`, exact: true }).getByRole("button", { name: "Save", exact: true }).click(); await until(async()=> (await fs.readFile(path.join(result.projects.A.root, relative),"utf8"))===text,"confirmed CodeMirror disk save"); }
}
async function uiCommit(p, message) {
  await panel().getByLabel("Commit message", { exact: true }).fill(message);
  const pending = page.waitForResponse(r => r.request().method() === "POST" && r.url().endsWith("/source-control/commit"));
  await panel().getByRole("button", { name: "Commit staged changes", exact: true }).click(); const r = await pending; assert.equal(r.status(), 200, await r.text()); const value = await r.json(); assert.equal(value.commit, checked(run("git", ["rev-parse", "HEAD"], p.root))); return value.commit;
}
async function manual() {
  const a = result.projects.A, b = result.projects.B; await select(a); await view("Source Control"); await panel().getByText("Working tree clean.", { exact: true }).waitFor(); await shot("clean-dark");
  await test("SC-BROWSER-01 CodeMirror save becomes real Git status", async () => {
    await openPath("src/manual.py"); const old = await fs.readFile(path.join(a.root, "src/manual.py"), "utf8"); a.manualChanged = old.replace("VALUE_2 = 2", "VALUE_2 = 200").replace("VALUE_30 = 30", "VALUE_30 = 3000");
    await edit("src/manual.py", a.manualChanged, true); assert.equal(await fs.readFile(path.join(a.root, "src/manual.py"), "utf8"), a.manualChanged); await fs.writeFile(path.join(a.root, "notes.txt"), "Additional staged evidence.\n");
    await view("Source Control"); await refresh(); await panel().getByRole("button", { name: /Msrc\/manual.py|M src\/manual.py/ }).click(); await panel().getByRole("button", { name: "Stage hunk 1", exact: true }).first().waitFor(); await shot("modified-hunk-diff");
  });
  await test("SC-BROWSER-02 hunk and file index semantics preserve unstaged code", async () => {
    const staged = page.waitForResponse(r => r.url().endsWith("/source-control/index") && r.request().method() === "POST"); await panel().getByRole("button", { name: "Stage hunk 1", exact: true }).first().click(); assert.equal((await staged).status(), 200);
    const cached = checked(run("git", ["diff", "--cached"], a.root)); assert.ok(cached.includes("VALUE_2 = 200")); assert.ok(!cached.includes("VALUE_30 = 3000")); assert.ok(checked(run("git", ["diff"], a.root)).includes("VALUE_30 = 3000"));
    await panel().getByRole("button", { name: "Stage notes.txt", exact: true }).click(); await until(async () => (await sc(a, `status?rootId=${a.rootId}`)).status.changes.find(c => c.path === "notes.txt")?.index === "A", "stage second file"); await refresh();
    await panel().getByRole("button", { name: "Unstage notes.txt", exact: true }).click(); await until(async () => (await sc(a, `status?rootId=${a.rootId}`)).status.changes.find(c => c.path === "notes.txt")?.index === "?", "unstage preserves file"); await refresh(); await panel().getByRole("button", { name: "Stage notes.txt", exact: true }).click(); await refresh(); await shot("staged-and-unstaged");
    a.firstCommit = await uiCommit(a, "P12 selected hunk and additional file"); const blob = checked(run("git", ["show", "HEAD:src/manual.py"], a.root)); assert.ok(blob.includes("VALUE_2 = 200")); assert.ok(!blob.includes("VALUE_30 = 3000")); assert.equal(await fs.readFile(path.join(a.root, "src/manual.py"), "utf8"), a.manualChanged);
    await refresh(); await panel().getByRole("button", { name: "Stage src/manual.py", exact: true }).click(); await refresh(); a.secondCommit = await uiCommit(a, "P12 remaining manual change");
  });
  await test("SC-BROWSER-03 branches protect dirty CodeMirror buffers", async () => {
    await refresh(); await panel().getByText("Branches", { exact: true }).click(); await panel().getByLabel("New local branch").fill("p12-local"); await panel().getByRole("button", { name: "Create and switch branch", exact: true }).click();
    await until(async () => checked(run("git", ["branch", "--show-current"], a.root)) === "p12-local", "local branch creation");
    await openPath("src/manual.py"); a.branchChanged = a.manualChanged + "EXTRA = 99\n"; await edit("src/manual.py", a.branchChanged, false); await view("Source Control"); await refresh();
    const branches = panel().locator("details").filter({ has: page.locator("summary", { hasText: /^Branches$/ }) }).first(); if ((await branches.getAttribute("open")) === null) await branches.locator("summary").click(); await branches.getByRole("button", { name: "main", exact: true }).click(); await panel().getByText(/Unsaved editor buffers are preserved/).waitFor(); assert.equal(checked(run("git", ["branch", "--show-current"], a.root)), "p12-local"); await shot("dirty-buffer-checkout-blocked");
    await openPath("src/manual.py"); await edit("src/manual.py", a.branchChanged, true); await view("Source Control"); await refresh(); await panel().getByRole("button", { name: "Stage src/manual.py", exact: true }).click(); await refresh(); a.branchCommit = await uiCommit(a, "P12 saved draft on feature branch"); await refresh(); await branches.getByRole("button", { name: "main", exact: true }).click(); await until(async () => checked(run("git", ["branch", "--show-current"], a.root)) === "main", "clean branch switch"); assert.equal(await fs.readFile(path.join(a.root, "src/manual.py"), "utf8"), a.manualChanged);
    await panel().getByText("Commit history", { exact: true }).click(); await panel().getByRole("button", { name: /P12 remaining manual change/ }).waitFor(); await shot("branch-and-commit-history");
  });
  await test("SC-BROWSER-06 reserved branch aliases are rejected without changing refs", async () => {
    await refresh();
    const branches = panel().locator("details").filter({ has: page.locator("summary", { hasText: /^Branches$/ }) }).first();
    if ((await branches.getAttribute("open")) === null) await branches.locator("summary").click();
    const original = checked(run("git", ["rev-parse", "HEAD"], a.root));
    await panel().getByLabel("New local branch").fill("Factory/p12-denied");
    const response = page.waitForResponse(r => r.url().endsWith("/source-control/branches") && r.request().method() === "POST");
    await panel().getByRole("button", { name: "Create and switch branch", exact: true }).click();
    assert.equal((await response).status(), 400);
    await panel().getByText(/Invalid or ASF-owned branch name/).waitFor();
    assert.equal(checked(run("git", ["rev-parse", "HEAD"], a.root)), original);
    assert.equal(checked(run("git", ["branch", "--show-current"], a.root)), "main");
    assert.notEqual(run("git", ["rev-parse", "--verify", "refs/heads/Factory/p12-denied"], a.root).exit, 0);
    await shot("reserved-case-alias-rejected");
  });
  await test("SC-BROWSER-04 project isolation persistence themes and narrow layout", async () => {
    await select(b); await view("Source Control"); await panel().getByText("Working tree clean.", { exact: true }).waitFor(); assert.ok(!(await panel().innerText()).includes("P12 remaining manual change")); await select(a); await view("Source Control"); await settled(); await page.reload(); await page.getByLabel("Open registered project").waitFor(); await settled(); await view("Source Control"); await panel().getByText("Working tree clean.", { exact: true }).waitFor();
    await page.getByLabel("Theme", { exact: true }).selectOption("light"); const loaded = panel().getByLabel("Git target version", { exact: true }); await loaded.waitFor(); assert.ok((await loaded.innerText()).includes("VALUE_2 = 200")); await shot("light-source-control"); await page.setViewportSize({ width: 1080, height: 850 }); await shot("narrow-source-control"); await loaded.scrollIntoViewIfNeeded(); await shot("narrow-source-diff"); await page.setViewportSize({ width: 1600, height: 1080 }); await page.getByLabel("Theme", { exact: true }).selectOption("dark");
  });
}
async function realMission() {
  const a = result.projects.A; a.before = run("python", ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"], a.root); assert.notEqual(a.before.exit, 0); assert.match(a.before.stderr, /test_nested.*FAIL/);
  if(inspectModel){
    await select(a);const recorded=await get(a,`runs/${a.runId}`);assert.equal(recorded.run.status,"completed");assert.deepEqual(recorded.sessions.map(s=>s.id),a.detail.sessions.map(s=>s.id));
  } else if(!retryModel) {
  for (const role of ["planner", "worker", "reviewer"]) await mutate(a, "agent-profiles", { name: `P12 ${role}`, providerAdapterId: role, modelId: "gpt-6.1-sol", reasoningEffort: "low", roles: [role], instructions: "Use the enforced sandbox and individual direct commands. Return the required role JSON with accurate evidence. Do not stage, commit, publish, install or change policy. For this small task use one implementation task including regression tests. A read-only Reviewer must examine the Rust-captured base/head/patch and actual tests; do not grant writes to inspect Git metadata.", permissions: {}, maxConcurrency: 1, timeoutSecs: 300, enabled: true, automaticDelegation: true, preferred: true });
  await select(a); await view("Orchestrator"); const chat = page.locator(".chat-panel:visible").last(); const creating = page.waitForResponse(r => r.request().method() === "POST" && r.url().endsWith("/conversations")); await chat.getByRole("button", { name: "New conversation", exact: true }).click(); a.conversationId = (await (await creating).json()).id;
  await chat.getByRole("button", { name: "Mission form", exact: true }).click(); a.objective = "Investigate incorrect interval-handling behavior in src/ranges.py. Identify the root cause, fix the implementation, and add regression tests for overlapping, nested, duplicate and boundary ranges. Preserve valid existing behavior. Run python -B -m unittest discover -s tests -v. Keep one bounded implementation task including tests. No network, publication, installations, policy changes or work outside the authorized task worktree. ASF owns commit capture, independent review and integration.";
  await chat.getByLabel("Mission objective", { exact: true }).fill(a.objective); await chat.getByLabel("Acceptance criteria, one per line", { exact: true }).fill("All original tests and new regression tests pass. A relevant source diff is independently reviewed and integrated.");
  const posted = page.waitForResponse(r => r.request().method() === "POST" && r.url().endsWith("/messages")); await chat.getByRole("button", { name: "Plan and start mission", exact: true }).click(); const r = await posted; assert.equal(r.status(), 202, await r.text()); a.sessionId = r.request().postDataJSON().workspace_session_id;
  a.runId = (await until(async () => (await get(a, `conversations/${a.conversationId}/missions`))[0], "persisted mission")).run_id; await note("mission.submitted", { run: a.runId, session: a.sessionId }); await shot("genuine-mission-planning");
  } else {
    await select(a);
    const profiles=await get(a,"agent-profiles");const worker=profiles.find(p=>p.name==="P12 worker");assert.ok(worker);
    const configuration={...Object.fromEntries(["name","description","providerAdapterId","modelId","reasoningEffort","roles","instructions","permissions","maxConcurrency","timeoutSecs","enabled","automaticDelegation","preferred"].map(key=>[key,worker[key]])),instructions:worker.instructions+" The previous attempt was correctly rejected for shell pipelines. Do not use pipes, here-strings, command chains, bundled commands or interpreter edit scripts. Inspect the preserved unapproved partial work independently. Use the native apply_patch tool for any justified file edit; this is a file-edit tool, not a shell command. Describe native tool use in summary, and report only actually executed standalone shell commands in commands. Run python -B -m unittest discover -s tests -v and git diff --check as individual commands. Do not stage, commit, change policy, publish or request broader permissions."};
    const revised=worker.instructions.includes("The previous attempt was correctly rejected") ? worker : await api(`/api/projects/${a.id}/agent-profiles/${worker.id}`,"PATCH",{expectedRevision:worker.revision,configuration});assert.deepEqual(revised.permissions,worker.permissions);assert.equal(revised.modelId,worker.modelId);a.retryProfile={id:worker.id,from:worker.revision,to:revised.revision,permissionsUnchanged:true};
    const original=await get(a,`runs/${a.runId}`);const task=original.tasks.find(t=>t.state==="failed"||t.state==="blocked");assert.ok(task);await note("task.explicit_retry_authorized",{task:task.id,previousAttempt:original.attempts.at(-1)?.id});await mutate(a,`tasks/${task.id}/retry`,{});await shot("retry-preserved-coding-mission");
  }
  await until(async () => { const d = await get(a, `runs/${a.runId}`); if (["failed", "blocked", "cancelled"].includes(d.run.status)) { a.failedDetail = execution(d); throw Error(`Genuine mission ${d.run.status}; retained evidence, no silent model retry`); } const w = d.sessions.find(s => s.role === "worker" && s.id>(a.priorWorker?.id??0) && s.outputMeta?.processId); if (w && !a.worker) { a.worker = { id: w.id, process: w.outputMeta, snapshot: w.executionSnapshot }; await note("worker.observed", a.worker); } return d.run.status === "completed" && d; }, "genuine coding review and integration", 900000).then(d => { a.detail = execution(d); });
  result.cli.availability = "Verified by genuine configured Planner, Worker and Reviewer invocations";
  a.scheduler = await get(a, `runs/${a.runId}/scheduler`); a.integrations = a.scheduler.integrations.filter(i => i.status === "published"); assert.equal(a.integrations.length, 1); a.integrationCommit = a.integrations[0].candidate_sha;
  const attempt = a.detail.attempts.find(t => t.review?.decision === "approve" && t.commitSha); assert.ok(attempt); a.evidence = await sc(a, `reviews/${attempt.id}`); assert.equal(a.evidence.patchHashVerified, true); assert.ok(a.evidence.actualPaths.includes("src/ranges.py")); assert.ok(a.evidence.actualPaths.some(p => p.startsWith("tests/"))); const reviewer = a.evidence.sessions.find(s => s.role === "reviewer"); assert.equal(reviewer.reviewContext?.capturedPatchSupplied, true); a.reviewer = reviewer;
  const checkout = path.join(result.fixture, inspectModel ? `independent-verification-${Date.now()}` : "independent-verification"); checked(run("git", ["clone", "--local", "--no-hardlinks", "--no-checkout", a.root, checkout], result.fixture)); checked(run("git", ["remote", "remove", "origin"], checkout)); checked(run("git", ["checkout", "--detach", a.integrationCommit], checkout)); a.verification = run("python", ["-B", "-m", "unittest", "discover", "-s", "tests", "-v"], checkout); checked(a.verification);
  const oracle = path.join(checkout, "independent_oracle.py"); await fs.writeFile(oracle, "from src.ranges import merge_ranges\nimport random\nr=random.Random(12)\nfor _ in range(500):\n spans=[tuple(sorted((r.randrange(-20,21),r.randrange(-20,21)))) for _ in range(r.randrange(15))]\n got=merge_ranges(spans)\n assert all(a<=b for a,b in got)\n assert all(got[i][1]<got[i+1][0] for i in range(len(got)-1))\n assert {p for a,b in spans for p in range(2*a,2*b+1)}=={p for a,b in got for p in range(2*a,2*b+1)}\nprint('500 independent coverage/order/non-overlap cases passed')\n"); a.oracle = run("python", ["-B", oracle], checkout); checked(a.oracle); a.diff = checked(run("git", ["diff", a.initialHead, a.integrationCommit, "--stat"], a.root)); await fs.writeFile(path.join(output, "integrated.patch"), checked(run("git", ["diff", "--binary", a.initialHead, a.integrationCommit], a.root)));
  await test("SC-MODEL-01 genuine read-only review of captured source diff", async () => { assert.ok(a.detail.sessions.filter(s => ["planner", "worker", "reviewer"].includes(s.role)).every(s => s.nativeHeader.includes("model: gpt-6.1-sol"))); assert.ok(a.detail.sessions.find(s => s.role === "reviewer" && s.nativeHeader.some(line=>line.startsWith("sandbox: read-only")))); assert.equal(a.verification.exit, 0); assert.equal(a.oracle.exit, 0); });
  if(inspectModel){await page.getByRole("button",{name:"Focus",exact:true}).click();await view("Orchestrator");const chat=page.locator(".chat-panel:visible").last();await chat.getByLabel("Conversation",{exact:true}).selectOption(String(a.conversationId));a.finalConversation=await until(async()=>{const messages=await get(a,`conversations/${a.conversationId}/messages?limit=100`);return messages.find(message=>message.content.startsWith(`Mission #${a.runId} is completed.`));},"current completed retry conversation outcome");await page.locator(".chat-panel:visible").last().getByText(a.finalConversation.content,{exact:true}).scrollIntoViewIfNeeded();await shot("current-completed-conversation");}
  await page.getByRole("button",{name:"Review",exact:true}).click();await view("Review"); const review = page.locator(".sc-review:visible").last(); await review.getByLabel("Recorded task attempt").selectOption(String(attempt.id)); await review.locator(".sc-file-list").getByRole("button", { name: "src/ranges.py", exact: true }).click(); await review.getByText(/Captured diff supplied: yes/).waitFor();const actualDiff=review.getByLabel("Git target version",{exact:true});await actualDiff.waitFor();assert.ok((await actualDiff.innerText()).includes("max("));await shot("genuine-review-evidence");await actualDiff.scrollIntoViewIfNeeded();await shot("genuine-source-diff");
  if(!inspectModel){await review.getByLabel("Human review comment").fill("Independently verified nested ranges and 500 coverage cases. Human feedback supplements the recorded automatic approval."); await review.getByRole("button", { name: "Save human feedback", exact: true }).click(); await review.getByText(/Human feedback saved/).waitFor();}else assert.ok(a.evidence.humanComments.length);await review.locator(".sc-comment").first().scrollIntoViewIfNeeded();await shot("durable-human-review");
  await view("Source Control"); await refresh(); const roots = await sc(a, "worktrees"); a.worktreeInspection = roots; assert.ok(roots.some(w => w.root.kind === "task_worktree" && w.root.read_only)); assert.ok(roots.some(w => w.root.kind === "integration_snapshot" && w.root.read_only)); const integrated = roots.find(w => w.root.kind === "integration_snapshot"); await panel().getByLabel("Repository / worktree").selectOption(integrated.root.id); await panel().getByText("Managed worktree \u00b7 inspection only", { exact: true }).waitFor(); await shot("managed-integration-worktree");
  await note("mission.independently_verified", { integration: a.integrationCommit, reviewer: reviewer.id, originalWorker: a.worker?.id });
}
async function remoteReads() {
  const a = result.projects.A;const existing=checked(run("git",["remote"],a.root));if(existing.split(/\r?\n/).includes("origin"))assert.equal(checked(run("git",["remote","get-url","origin"],a.root)),"https://github.com/OmegaMc1331/agentic-software-factory.git");else checked(run("git", ["remote", "add", "origin", "https://github.com/OmegaMc1331/agentic-software-factory.git"], a.root));
  await select(a); await view("Source Control"); const details = panel().locator("details").filter({ has: page.locator("summary", { hasText: /^GitHub and authorized delivery$/ }) }).first(); await details.locator("summary").click();
  const remoteResponse = page.waitForResponse(r => r.url().includes("/source-control/github?") && r.request().method() === "GET"); await panel().getByRole("button", { name: "Read authenticated remote", exact: true }).click(); const response = await remoteResponse;
  result.remoteRead = { status: response.status(), data: await response.json(), writes: 0 }; assert.equal(response.status(), 200, JSON.stringify(result.remoteRead)); assert.equal(result.remoteRead.data.repository.fullName.toLowerCase(), "omegamc1331/agentic-software-factory");
  const pull = result.remoteRead.data.pullRequests.items[0]; if (pull) { await panel().getByRole("button", { name: new RegExp(`^#${pull.number} `) }).click(); await panel().getByRole("heading", { name: `Pull Request #${pull.number}`, exact: true }).waitFor(); result.remoteDetail = await sc(a, `github/pulls/${pull.number}`); }
  await shot("actual-github-read-only");
  if (model) { const started=performance.now();await panel().getByLabel("Mission ID for delivery").fill(String(a.runId)); const pending = page.waitForResponse(r => r.url().endsWith(`/source-control/delivery/${a.runId}`) && r.request().method() === "GET"); await panel().getByRole("button", { name: /Preview delivery/ }).click(); const r = await pending; result.deliveryPreview = { status: r.status(), data: await r.json(),durationMs:performance.now()-started,measurementScope:"UI action through acknowledged preview body" }; assert.equal(r.status(), 200); assert.equal(result.deliveryPreview.data.publishedByPreview, false);assert.equal(result.deliveryPreview.data.changedFilesComplete,true);assert.ok(result.deliveryPreview.data.changedFiles.includes("src/ranges.py"));await shot("delivery-preview-no-publication"); }
  result.tests.push({ name: "SC-GITHUB-READ actual authorized repository and PR data, no remote mutation", status: "PASS" });
}
async function retainedWork() {
  for (const name of ["Parallel","Conflict"]) await test(`SC-RETAINED ${name} ${result.projects[name].controlled ? "current controlled scheduler" : "retained genuine P08"} evidence through P12 APIs and browser`,async()=>{
    const p=result.projects[name];await select(p);if(p.controlled&&!p.execution)await controlledMission(p,name);await view("Source Control");await refresh();p.worktrees=await sc(p,"worktrees");assert.ok(p.worktrees.filter(w=>w.root.kind==="task_worktree").length>=2);assert.equal(checked(run("git",["rev-parse","HEAD"],p.root)),p.initialHead);
    p.scheduler=await get(p,"runs/1/scheduler");assert.deepEqual(p.scheduler.integrations.map(i=>[i.id,i.status,i.candidate_sha]),p.sourceIntegrationRecords.map(i=>[i.id,i.status,i.candidate_sha]));
    const taskRoot=p.worktrees.find(w=>w.root.kind==="task_worktree");await panel().getByLabel("Repository / worktree").selectOption(taskRoot.root.id);await panel().getByText("Managed worktree \u00b7 inspection only",{exact:true}).waitFor();await shot(`retained-${name.toLowerCase()}-worktrees`);
    await page.getByRole("button",{name:"Review",exact:true}).click();await view("Review");const r=page.locator(".sc-review:visible").last();p.reviews=await sc(p,"reviews?runId=1");const item=p.reviews.items.find(i=>i.attempt.status==="integrated"||i.attempt.status==="approved");assert.ok(item);await r.getByLabel("Recorded task attempt").selectOption(String(item.attempt.id));p.review=await sc(p,`reviews/${item.attempt.id}`);assert.ok(p.review.actualPaths.length>0);assert.equal(p.review.attempt.review.decision,"approve");await r.locator(".sc-file-list").getByRole("button",{name:p.review.actualPaths[0],exact:true}).click();const target=r.getByLabel("Git target version",{exact:true});await target.waitFor();await target.scrollIntoViewIfNeeded();await shot(`retained-${name.toLowerCase()}-review`);
    if(name==="Conflict"){
      await r.getByText("Historical conflict retained",{exact:true}).scrollIntoViewIfNeeded();assert.ok(p.review.integration.some(i=>i.status==="conflict"));assert.ok(p.scheduler.revisions.length>=2);await shot("retained-conflict-replan-recovery");
      await view("Source Control");await refresh();const conflicted=p.worktrees.find(w=>w.status.changes.some(c=>c.conflict));assert.ok(conflicted,"Preserved real integration conflict worktree");await panel().getByLabel("Repository / worktree").selectOption(conflicted.root.id);const change=conflicted.status.changes.find(c=>c.conflict);await panel().locator(".sc-file-list button[title]").filter({hasText:change.path}).first().click();await panel().getByRole("heading",{name:"Actual unmerged index stages",exact:true}).waitFor();p.conflict=await sc(p,`conflicts?rootId=${conflicted.root.id}&path=${encodeURIComponent(change.path)}`);assert.equal(p.conflict.readOnly,true);await shot("actual-preserved-merge-conflict");
    }
    assert.equal(checked(run("git",["status","--porcelain"],p.root)),"");
  });
}
async function controlledMission(p,name){
  await view("Orchestrator");const chat=page.locator(".chat-panel:visible").last();const creating=page.waitForResponse(r=>r.request().method()==="POST"&&r.url().endsWith("/conversations"));await chat.getByRole("button",{name:"New conversation",exact:true}).click();p.conversationId=(await(await creating).json()).id;
  const tasks=name==="Parallel" ? [{title:"A",objective:"WRITE A",dependencies:[]},{title:"B",objective:"WRITE B",dependencies:[]},{title:"Dependent",objective:"WRITE dependent",dependencies:["T1","T2"]}] : [{title:"Conflicting A",objective:"WRITE conflictA",dependencies:[]},{title:"Conflicting B",objective:"WRITE conflictB",dependencies:[]}];
  await chat.getByRole("button",{name:"Visual plan",exact:true}).click();await chat.getByLabel("Mission objective",{exact:true}).fill(`Controlled ${name} scheduler UI qualification; no model execution. Preserve and independently verify all contributions.`);await chat.getByLabel("Scope",{exact:true}).fill("Disposable fixture source files only");await chat.getByLabel("Constraints",{exact:true}).fill("Bounded unfinished-task replanning is authorized. No publication or policy change.");
  for(let i=0;i<tasks.length;i++){const id=`T${i+1}`,task=tasks[i];await chat.getByRole("button",{name:"Add task",exact:true}).click();await chat.getByLabel(`Title ${id}`,{exact:true}).fill(task.title);await chat.getByLabel(`Objective ${id}`,{exact:true}).fill(task.objective);await chat.getByLabel(`Acceptance ${id}`,{exact:true}).fill("Actual source changes, controlled review and combined verification required");await chat.getByLabel(`Dependencies ${id}`,{exact:true}).fill(task.dependencies.join(","));}
  const posting=page.waitForResponse(r=>r.request().method()==="POST"&&r.url().endsWith("/messages"));await chat.getByRole("button",{name:"Validate and launch plan",exact:true}).click();const response=await posting;assert.equal(response.status(),202,await response.text());p.runId=(await until(async()=>(await get(p,`conversations/${p.conversationId}/missions`))[0],"controlled mission identity")).run_id;
  const detail=await until(async()=>{const d=await get(p,`runs/${p.runId}`);if(["failed","blocked","cancelled"].includes(d.run.status)){p.failure=execution(d);throw Error(`Controlled scheduler ${name} ${d.run.status}`);}return d.run.status==="completed"&&d;},"controlled scheduler completion",180000);
  p.execution=execution(detail);const intervals=detail.sessions.filter(s=>s.role==="worker").map(s=>({id:s.id,start:s.outputMeta.processStartedUnixMs,end:s.outputMeta.processFinishedUnixMs}));p.controlledPeak=intervals.reduce((n,a)=>Math.max(n,intervals.filter(b=>b.start<=a.start&&b.end>a.start).length),0);assert.equal(p.controlledPeak,2);
  p.sourceIntegrationRecords=(await get(p,`runs/${p.runId}/scheduler`)).integrations;
  const integration=detail.run.integration_sha ?? p.sourceIntegrationRecords.filter(i=>i.status==="published").at(-1).candidate_sha;
  const checkout=path.join(result.fixture,`verify-controlled-${name}`);checked(run("git",["clone","--local","--no-hardlinks","--no-checkout",p.root,checkout],result.fixture));checked(run("git",["remote","remove","origin"],checkout));checked(run("git",["checkout","--detach",integration],checkout));p.independentVerification=run("python",["-B","-c",name==="Parallel"?"from part_A import VALUE as a; from part_B import VALUE as b; from part_dependent import VALUE as c; assert (a,b,c)==(1,1,1); print('Both contributions and dependent result verified')":"from shared import FLAGS; assert FLAGS=={'base': True, 'A': True, 'B': True}; print('Both conflicting contributions retained by verified repair')"],checkout);checked(p.independentVerification);
}
async function measure() {
  const p=result.projects.A;result.performance=[];result.mode="synthetic-source-data-real-Git-and-production-API-performance";
  const stats=values=>({samples:values.length,minMs:Math.min(...values),medianMs:[...values].sort((a,b)=>a-b)[Math.floor(values.length/2)],maxMs:Math.max(...values)});
  async function timing(action,n=5){const times=[];for(let i=0;i<n;i++){const start=performance.now();await action(i);times.push(performance.now()-start);}return stats(times);}
  let previous=0;
  for(const count of [100,1000,6000]){
    await fs.mkdir(path.join(p.root,"generated"),{recursive:true});for(let i=previous;i<count;i++)await fs.writeFile(path.join(p.root,`generated/file_${String(i).padStart(5,"0")}.txt`),`fixture ${i}\n`);previous=count;
    checked(run("git",["add","generated"],p.root));checked(run("git",["commit","-qm",`performance fixture ${count} files`],p.root));
    const sample={generatedFiles:count,cleanStatus:await timing(()=>sc(p,`status?rootId=${p.rootId}`)),branches:await timing(()=>sc(p,`branches?rootId=${p.rootId}`)),history:await timing(()=>sc(p,`history?rootId=${p.rootId}&skip=0&limit=25`))};
    for(let i=0;i<Math.min(count,200);i++)await fs.writeFile(path.join(p.root,`generated/file_${String(i).padStart(5,"0")}.txt`),`changed fixture ${i}\n`);
    sample.changedFiles=Math.min(count,200);sample.changedStatus=await timing(()=>sc(p,`status?rootId=${p.rootId}`));sample.diff=await timing(()=>sc(p,"diff",{rootId:p.rootId,diff:{path:"generated/file_00000.txt",comparison:"working_index",base:null,target:null}}));
    sample.hunkStage=await timing(async()=>{const d=await sc(p,"diff",{rootId:p.rootId,diff:{path:"generated/file_00000.txt",comparison:"working_index",base:null,target:null}});assert.ok(d.hunks.length);await sc(p,"index",{rootId:p.rootId,key:crypto.randomUUID(),request:{expectedGeneration:d.generation,paths:[d.path],unstage:false,hunk:d.hunks[0].id,diffVersion:d.version,trustPrograms:false}});const s=(await sc(p,`status?rootId=${p.rootId}`)).status;await sc(p,"index",{rootId:p.rootId,key:crypto.randomUUID(),request:{expectedGeneration:s.generation,paths:[d.path],unstage:true,hunk:null,diffVersion:null,trustPrograms:false}});},3);
    for(let i=0;i<Math.min(count,200);i++)await fs.writeFile(path.join(p.root,`generated/file_${String(i).padStart(5,"0")}.txt`),`fixture ${i}\n`);
    sample.commandScope="Hunk sample includes preview, stage, fresh status and unstage; no latency claimed for a single sub-operation";
    result.performance.push(sample);await persist();
  }
  let tree=checked(run("git",["rev-parse","HEAD^{tree}"],p.root));let parent=checked(run("git",["rev-parse","HEAD"],p.root));
  for(let i=0;i<250;i++)parent=checked(run("git",["commit-tree",tree,"-p",parent,"-m",`actual generated history ${i}`],p.root));checked(run("git",["update-ref","refs/heads/main",parent],p.root));
  result.historyPagination=await timing(i=>sc(p,`history?rootId=${p.rootId}&skip=${i*25}&limit=25`));
  await fs.writeFile(path.join(p.root,"generated/file_00000.txt"),"explicit performance commit\n");
  let state=(await sc(p,`status?rootId=${p.rootId}`)).status;
  result.fileStage=await timing(async()=>{state=await sc(p,"index",{rootId:p.rootId,key:crypto.randomUUID(),request:{expectedGeneration:state.generation,paths:["generated/file_00000.txt"],unstage:false,hunk:null,diffVersion:null,trustPrograms:false}});},1);
  result.commitCreation=await timing(async()=>{const committed=await sc(p,"commit",{rootId:p.rootId,key:crypto.randomUUID(),expectedGeneration:state.generation,message:"Explicit local performance commit",trustPrograms:false});assert.equal(committed.commit,checked(run("git",["rev-parse","HEAD"],p.root)));state=committed.status;},1);
  result.branchCheckout=await timing(async()=>{state=await sc(p,"branches",{rootId:p.rootId,key:crypto.randomUUID(),expectedGeneration:state.generation,name:"p12-perf-local",create:true,trustPrograms:false,dirtyBuffers:false});},1);
  for(let i=0;i<3;i++)checked(run("git",["worktree","add","-b",`factory/p12-perf-${i}`,path.join(p.root,`.factory/worktrees/p12-perf-${i}`),"HEAD"],p.root));
  result.worktreeEnumeration=await timing(()=>sc(p,"worktrees"),3);
  result.reviewList=await timing(()=>sc(p,"reviews?before=9999999"));
  await select(p);await view("Source Control");await refresh();await shot("large-repository-source-control");
  result.memorySample=JSON.parse(checked(run("powershell.exe",["-NoProfile","-Command",`Get-Process -Id ${Number(child.pid)} | Select-Object Id,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`],result.fixture)));
  result.browserPerformance=await page.evaluate(()=>({paint:performance.getEntriesByType("paint").map(entry=>({name:entry.name,startTimeMs:entry.startTime})),heap:performance.memory?{usedJSHeapSize:performance.memory.usedJSHeapSize,totalJSHeapSize:performance.memory.totalJSHeapSize,jsHeapSizeLimit:performance.memory.jsHeapSizeLimit}:null}));
  result.browserPerformance.scope="One Chromium sample after fixture navigation; paint is initial navigation, not Source Control-specific render time; heap is JavaScript only";
  result.databaseBytes=(await fs.stat(path.join(p.root,".factory/db.sqlite3"))).size;result.tests.push({name:"SC-PERFORMANCE bounded production Git fixtures and API samples",status:"PASS"});
}
try {
  await fixtures(); await start(); if(performanceOnly) await measure(); else if (retained) await retainedWork(); else if (model) await realMission(); else await manual(); if(!retained&&!performanceOnly) await remoteReads(); assert.deepEqual(result.pageErrors, []); assert.deepEqual(result.publicationRequests,[]); result.status = "PASS";
} catch (error) { result.status = "FAILED"; result.error = String(error.stack ?? error); if (page) try { result.visibleFailure = (await page.locator("body").innerText()).slice(0, 20000);const file="screenshots/failure.png";await page.screenshot({path:path.join(output,file)});result.screenshots.push({file,sha256:hash(await fs.readFile(path.join(output,file)))}); } catch {} process.exitCode = 1; }
finally {
  for(const {server,waiting} of barriers){waiting.splice(0).forEach(socket=>socket.end("G"));await new Promise(resolve=>server.close(resolve));}
  await browser?.close(); await inspector?.dispose(); if (child?.exitCode === null) { const exited = new Promise(resolve => child.once("exit", resolve)); child.kill(); await exited; }
  result.finishedAt = new Date().toISOString(); result.cleanup = { ownedServerStarted:!!child,ownedServerExited:child ? child.exitCode !== null || child.signalCode !== null : null, remoteWrites: 0 }; await persist(); console.log(JSON.stringify({ status: result.status, evidence: recordPath, error: result.error }));
}
