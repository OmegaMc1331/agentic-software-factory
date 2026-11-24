import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import {spawn,spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const output=process.env.ASF_P08_OUTPUT??path.join(root,'.factory/v2/p08/acceptance');
await fs.mkdir(path.join(output,'screenshots'),{recursive:true});
const recordFile=path.join(output,'results.json');
let prior;try {prior=JSON.parse(await fs.readFile(recordFile,'utf8'));}catch(e){if(e.code!=='ENOENT')throw e;}
if(prior && process.env.ASF_P08_RESUME!=='1')throw new Error('Existing acceptance retained; choose explicit resume or a new output.');
const binary=process.env.ASF_P08_BINARY??path.join(root,'.factory/v2/p08/run/factory-p08.exe');
const cli = process.env.ASF_P08_CODEX ?? 'codex';
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const require=createRequire(path.join(root,'apps/dashboard/package.json'));
const {chromium}=require('playwright');
const result=prior??{schema:1,machine:{platform:os.platform(),release:os.release(),cpu:os.cpus()[0].model,logicalCpus:os.cpus().length,ramBytes:os.totalmem(),node:process.version},started:new Date().toISOString(),fixture:await fs.mkdtemp(path.join(os.tmpdir(),'asf-p08-real-')),tests:[],scenarios:{},screenshots:[],errors:[]};
const binaryHash=hash(await fs.readFile(binary));
if(prior)assert.equal(prior.binarySha256,binaryHash,'Resume uses the same executable');
Object.assign(result,{binary,binarySha256:binaryHash,scriptSha256:hash(await fs.readFile(fileURLToPath(import.meta.url)))});
const persist=()=>fs.writeFile(recordFile,JSON.stringify(result,null,2));
function run(program,args,cwd){const begin=performance.now();const r=spawnSync(program,args,{cwd,encoding:'utf8',windowsHide:true,timeout:60000,env:{...process.env,PYTHONDONTWRITEBYTECODE:'1'}});return{program,args,cwd,exit:r.status,stdout:r.stdout??'',stderr:r.stderr??'',ms:performance.now()-begin,error:r.error?.message};}
function ok(r){assert.equal(r.exit,0,JSON.stringify(r));return r.stdout.trim();}
result.sourceCommit=ok(run('git',['rev-parse','HEAD'],root));result.cliVersion=ok(run(cli,['--version'],root));
function profiles(){
 let config='[runtime]\nmax_parallel_tasks = 2\n[scheduler]\nmax_parallel_tasks = 2\nmax_project_tasks = 2\nmax_replans = 2\nmax_total_tasks = 32\nmax_duration_secs = 1800\n[[integration_checks]]\nprogram = "python"\nargs = ["-m","unittest","discover","-s","tests","-v"]\ntimeout_secs = 30\n';
 for(const [name,role,preferred,capacity] of [['planner','planner',true,1],['worker-a','worker',true,1],['worker-b','worker',false,1],['reviewer','reviewer',true,2]]){
 const args=['exec','--ignore-user-config','--ephemeral','--disable','apps','--sandbox',role==='worker'?'workspace-write':'read-only','-c','windows.sandbox=mxc','-c','allow_login_shell=false','--color','never'];
 config+=`\n[agents.${name}]\nkind = "codex"\ncommand = ${JSON.stringify(cli)}\nargs = ${JSON.stringify(args)}\nmax_concurrency = ${capacity}\n[agents.${name}.env]\nPYTHONDONTWRITEBYTECODE = "1"\n[[role_assignments]]\nrole = "${role}"\nagent = "${name}"\npreferred = ${preferred}\n`;
 }
 for(const role of ['planner','worker','reviewer'])config+=`\n[policies.roles.${role}.filesystem]\nread = ["**"]\nwrite = ${role==='worker'?'["src/**", "tests/**"]':'[]'}\n[policies.roles.${role}.commands]\nmode = "restricted"\nallow = ["python", "python3", "git", "rg", "Get-Content", "Set-Content", "Get-ChildItem", "Get-Location", "Test-Path", "powershell", "powershell.exe"]\ndeny = ["curl", "Invoke-WebRequest", "cmd", "bash", "sh"]\n[policies.roles.${role}.network]\nmode = "deny"\n`;
 return config;
}
async function prepare(name,files){
 if(result.scenarios[name])return result.scenarios[name];
 const folder=path.join(result.fixture,name);await fs.mkdir(folder,{recursive:true});
 for(const [file,content] of Object.entries({...files,'.gitignore':'.factory/\n__pycache__/\n*.pyc\n','src/__init__.py':''})) {await fs.mkdir(path.dirname(path.join(folder,file)),{recursive:true});await fs.writeFile(path.join(folder,file),content);}
 await fs.mkdir(path.join(folder,'tests'),{recursive:true});
 ok(run('git',['init','-q','-b','main'],folder));ok(run('git',['config','user.name','ASF P08'],folder));ok(run('git',['config','user.email','p08@example.invalid'],folder));ok(run('git',['add','.'],folder));ok(run('git',['commit','-qm','controlled initial application'],folder));ok(run(binary,['init'],folder));
 await fs.writeFile(path.join(folder,'.factory/config.toml'),profiles());
 const item={root:folder,initialHead:ok(run('git',['rev-parse','HEAD'],folder)),baseline:run('python',['-m','unittest','discover','-s','tests','-v'],folder),samples:[]};assert.equal(item.baseline.exit,0);
 result.scenarios[name]=item;await persist();return item;
}
await prepare('Parallel',{'src/validation.py':'def validate_request(value):\n    return value\n','src/events.py':'def render_event(name, payload):\n    return str(payload)\n','src/calculator.py':'def ceil_div(value, divisor):\n    if divisor <= 0:\n        raise ValueError("positive divisor required")\n    return value // divisor + 1\n','src/strings.py':'def slug(value):\n    return value\n','tests/test_baseline.py':'import unittest\nfrom src.validation import validate_request\nfrom src.calculator import ceil_div\nclass Baseline(unittest.TestCase):\n    def test_valid_request(self): self.assertEqual(validate_request({"id":1,"name":"Ada"}), {"id":1,"name":"Ada"})\n    def test_positive_division(self): self.assertEqual(ceil_div(7,3),3)\n'});
await prepare('Conflict',{'src/features.py':'def flags():\n    return {"base": True}\n','tests/test_baseline.py':'import unittest\nfrom src.features import flags\nclass Baseline(unittest.TestCase):\n    def test_base(self): self.assertTrue(flags()["base"])\n'});
await prepare('Repair',{'src/protocol.py':'def encode(value):\n    return str(value)\n','src/client.py':'from src.protocol import encode\ndef request(value):\n    return encode(value).upper()\n','tests/test_baseline.py':'import unittest\nfrom src.client import request\nclass Baseline(unittest.TestCase):\n    def test_client(self): self.assertEqual(request("hello"),"HELLO")\n'});
const port=await new Promise(resolve=>{const socket=net.createServer();socket.listen(0,'127.0.0.1',()=>{const n=socket.address().port;socket.close(()=>resolve(n));});});
const base=`http://127.0.0.1:${port}`;result.base=base;
const log=await fs.open(path.join(output,`server-${Date.now()}.log`),'w');
const child=spawn(binary,['start','--no-browser','--port',String(port),'--max-active-tasks','2','--registry',path.join(result.fixture,'registry.sqlite3'),'--allow-project-root',result.fixture],{cwd:result.scenarios.Parallel.root,windowsHide:true,stdio:['ignore',log.fd,log.fd]});await log.close();
let browser,context,page;
async function eventually(fn,label,ms=45000){const end=Date.now()+ms;while(Date.now()<end){const v=await fn();if(v)return v;await new Promise(resolve=>setTimeout(resolve,300));}throw new Error(`Timeout: ${label}`);}
async function auth(){const response=await context.request.post(base+'/api/auth/session',{headers:{Origin:base},data:{}});assert.ok(response.ok(),await response.text());return(await response.json()).csrfToken;}
async function mutate(url,data,method='POST',expected=200){const response=await context.request.fetch(base+url,{method,data,headers:{Origin:base,'X-Factory-CSRF':await auth()}});assert.equal(response.status(),expected,await response.text());return expected===204?null:response.json();}
async function get(item,suffix){const response=await context.request.get(`${base}/api/projects/${item.project}/${suffix}`);assert.ok(response.ok(),await response.text());return response.json();}
async function shot(name){await page.screenshot({path:path.join(output,'screenshots',name+'.png'),fullPage:true});if(!result.screenshots.includes(name))result.screenshots.push(name);}
const panel=()=>page.locator('.chat-panel:visible').first();
async function view(label){await page.getByRole('button',{name:'Views',exact:true}).click();await page.getByRole('menuitem',{name:label,exact:true}).click();}
async function select(item){await page.getByLabel('Open registered project').selectOption(item.project);await view('Orchestrator');}
async function conversation(item){await select(item);const pending=page.waitForResponse(r=>r.request().method()==='POST' && /\/conversations$/.test(new URL(r.url()).pathname));await panel().getByRole('button',{name:'New conversation',exact:true}).click();const response=await pending;assert.ok(response.ok(),await response.text());item.conversation=(await response.json()).id;}
async function submit(item,objective,plan){
 await conversation(item);item.objective=objective;item.submittedAt=new Date().toISOString();
 const pending=page.waitForResponse(r=>r.request().method()==='POST' && /\/messages$/.test(new URL(r.url()).pathname));
 if(plan){
   await panel().getByRole('button',{name:'Visual plan',exact:true}).click();
   await panel().getByLabel('Mission objective').fill(objective);
   await panel().getByLabel('Scope',{exact:true}).fill('src/** and tests/** only');
   await panel().getByLabel('Constraints',{exact:true}).fill('Preserve sandbox and policy. Autonomous unfinished-plan repair authorized.');
   for(let i=0;i<plan.tasks.length;i++){
     const task=plan.tasks[i],id=`T${i+1}`;
     await panel().getByRole('button',{name:'Add task',exact:true}).click();
     await panel().getByLabel(`Title ${id}`,{exact:true}).fill(task.title);
     await panel().getByLabel(`Objective ${id}`,{exact:true}).fill(task.objective);
     await panel().getByLabel(`Acceptance ${id}`,{exact:true}).fill(task.acceptanceCriteria.join('; '));
     await panel().getByLabel(`Dependencies ${id}`,{exact:true}).fill(task.dependencies.map(dependency=>`T${plan.tasks.findIndex(task=>task.id===dependency)+1}`).join(','));
   }
   await shot('visual-plan-before-launch');
   await panel().getByRole('button',{name:'Validate and launch plan',exact:true}).click();
 } else {
   await panel().getByLabel('Message to orchestrator').fill(objective);await panel().getByRole('button',{name:'Send',exact:true}).click();
 }
 const response=await pending;assert.equal(response.status(),202,await response.text());item.submission=await response.json();
 await view('Progress');await shot('mission-planning');await persist();
}
async function awaitMission(name,item){
 let captured=false;
 await eventually(async()=>{
 const missions=await get(item,`conversations/${item.conversation}/missions`);const mission=missions[0];if(!mission)return false;item.mission=mission;
 const active=mission.sessions.filter(session=>session.role==='worker'&&session.status==='running');
 item.samples.push({at:new Date().toISOString(),status:mission.status,revision:mission.plan_revision,active:active.map(session=>({id:session.id,agent:session.agent,task:session.task_id})),tasks:mission.tasks.map(task=>({id:task.id,state:task.state}))});
 if(active.length>=2&&!captured){await shot(name.toLowerCase()+'-parallel-workers');captured=true;const processDetail=await get(item,`runs/${mission.run_id}`);const pids=processDetail.sessions.filter(session=>active.some(worker=>worker.id===session.id)).map(session=>session.outputMeta?.processId).filter(Number.isInteger);if(pids.length>=2)item.actualProcessObservation=run('powershell.exe',['-NoProfile','-NonInteractive','-Command',`@(Get-Process -Id ${pids.join(',')} -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,StartTime)|ConvertTo-Json -Compress`],item.root);const roots=await get(item,'files/roots');item.managedRoots=roots.roots.filter(root=>root.kind==='task_worktree');for(const managed of item.managedRoots.slice(0,2)){const snapshot=await get(item,`files/content?root_id=${managed.id}&path=src/${name==='Parallel'?'validation.py':'features.py'}`);assert.equal(snapshot.read_only,true);const denied=await mutate(`/api/projects/${item.project}/files/content`,{file:snapshot.file,expected_revision:snapshot.revision,content:snapshot.content},'PUT',403);assert.equal(denied.code,'worktree_read_only');}}
 if(active.length>=2&&!item.kernelOverlap){const detail=await get(item,`runs/${mission.run_id}`);const pids=detail.sessions.filter(session=>active.some(worker=>worker.id===session.id)).map(session=>session.outputMeta?.processId).filter(Number.isInteger);if(pids.length>=2){const observation=run('powershell.exe',['-NoProfile','-NonInteractive','-Command',`@(Get-Process -Id ${pids.join(',')} -ErrorAction SilentlyContinue | Select-Object Id,ProcessName,StartTime)|ConvertTo-Json -Compress`],item.root);try{const records=JSON.parse(observation.stdout);if(Array.isArray(records)&&records.length>=2){item.actualProcessObservation=observation;item.kernelOverlap=true;}}catch{}}}
 if(mission.integrations?.some(record=>record.status==='conflict')&&!item.conflictCapture){await shot(name.toLowerCase()+'-conflict');item.conflictCapture=true;}
 if((mission.plan_revision??1)>1&&!item.replanCapture){await shot(name.toLowerCase()+'-replan');item.replanCapture=true;}
 if(item.samples.length%5===0)await persist();return mission.terminal;
 },name+' real mission completion',1800000);
 item.finishedAt=new Date().toISOString();item.elapsedMs=Date.parse(item.finishedAt)-Date.parse(item.submittedAt);
 item.detail=await get(item,`runs/${item.mission.run_id}`);item.scheduler=await get(item,`runs/${item.mission.run_id}/scheduler`);
 await shot(name.toLowerCase()+'-final');await persist();
 assert.equal(item.mission.status,'completed',JSON.stringify(item.mission));
 const workers=item.detail.sessions.filter(session=>session.role==='worker');
 item.intervals=workers.map(session=>({id:session.id,agent:session.agent,task:session.taskId,pid:session.outputMeta?.processId,start:session.outputMeta?.processStartedUnixMs,end:session.outputMeta?.processFinishedUnixMs,cwd:item.detail.attempts.find(attempt=>attempt.id===session.attemptId)?.worktreePath}));
 assert.ok(workers.every(session=>session.outputMeta?.processTreeSupervised),"Owned process supervision must be established");
 assert.ok(item.intervals.every(interval=>interval.pid && interval.start && interval.end>=interval.start), "Real supervised process intervals required");
 item.peak=item.intervals.reduce((maximum,entry)=>Math.max(maximum,item.intervals.filter(other=>other.start<=entry.start&&other.end>entry.start).length),0);
 item.patch=ok(run('git',['diff',item.initialHead,item.mission.integration_sha,'--','src','tests'],item.root));assert.ok(item.patch.trim());
 const verification=path.join(result.fixture,`verify-${name}-${Date.now()}`);ok(run('git',['clone','-q','--no-hardlinks','--single-branch','--branch',`factory/run-${item.mission.run_id}`,item.root,verification],result.fixture));
 item.independent=run('python',['-m','unittest','discover','-s','tests','-v'],verification);assert.equal(item.independent.exit,0,JSON.stringify(item.independent));
 const roots=await get(item,'files/roots');item.integrationRoot=roots.roots.find(root=>root.kind==='integration_snapshot');assert.ok(item.integrationRoot?.read_only,'Published integration snapshot must remain inspectable without write authority');
 item.mainHead=ok(run('git',['rev-parse','HEAD'],item.root));assert.equal(item.mainHead,item.initialHead);assert.equal(ok(run('git',['status','--porcelain'],item.root)),'');
 assert.equal(item.scheduler.activeClaims,0);assert.equal(ok(run('git',['remote'],item.root)),'');
 for(const attempt of item.detail.attempts.filter(attempt=>attempt.status==='approved'&&attempt.operation!=='review')){assert.equal(attempt.review.decision,'approve');assert.ok(attempt.evidence.diffPatch?.trim());}
 await persist();
}
async function test(name,fn){const start=performance.now();try{await fn();result.tests.push({name,status:'passed',ms:performance.now()-start});console.log('PASS '+name);}catch(error){result.tests.push({name,status:'failed',error:String(error)});throw error;}finally{await persist();}}
const task=(id,title,objective,dependencies=[])=>({id,title,objective,dependencies,acceptanceCriteria:['Source implements the objective','Run real unittest validation; report failures honestly'],role:'worker',operation:'implement'});
try{
 await eventually(async()=>{try{return(await fetch(base+'/api/health')).ok;}catch{return false;}},'backend');
 browser=await chromium.launch({channel:'msedge',headless:true});context=await browser.newContext({viewport:{width:1440,height:900},colorScheme:'dark',reducedMotion:'reduce'});page=await context.newPage();page.on('pageerror',error=>result.errors.push(String(error)));
 await auth();
 const registered=await (await context.request.get(base+'/api/projects')).json();
 const canonical=value=>value.replace(/^\\\\\?\\/,'').replaceAll('\\','/').toLowerCase();
 for(const [name,item]of Object.entries(result.scenarios)){
   const owning=registered.find(entry=>canonical(entry.canonical_root)===canonical(item.root));
   if(owning)item.project=owning.project_id;
   if(!item.project){const registration=await mutate('/api/projects/register',{path:item.root,display_name:name,trust_project:true},'POST',201);item.project=registration.project_id;}
   await mutate(`/api/projects/${item.project}/open`,{});
 }
 await page.goto(base+'/workspace.html');await page.getByLabel('Open registered project').waitFor();await page.getByLabel('Theme',{exact:true}).selectOption('dark');
 await test('A/B/F: real Planner, four ready features, cap two, dependent combined tests',async()=>{
 const item=result.scenarios.Parallel;
 if(!item.conversation)await submit(item,'Implement four independent improvements in this disposable Python project, with four independent Worker tasks and a final dependent combined-test task: (1) src/validation.py: validate_request must accept a dict with positive integer id (reject bool) and nonempty string name, reject invalid input with ValueError, preserve valid input; add tests/test_validation.py. (2) src/events.py: render_event(name,payload) must produce deterministic JSON with event and payload fields using stdlib; add tests/test_events.py. (3) src/calculator.py: fix ceil_div for exact multiples, zero and negative integer values with positive divisor, preserve ValueError for nonpositive divisor; add tests/test_calculator.py. (4) src/strings.py: slug strips, lowercases and collapses whitespace to hyphens; add tests/test_strings.py. Keep these four tasks independent and each confined to its module and test file. (5) After all four are integrated, add tests/test_combined.py exercising their combined behavior and run the complete suite. ASF owns reviews and integration. Use only src/** and tests/**, run python -m unittest discover -s tests -v. Do not stage, commit, publish, push, install, request broader permissions, or bypass any denial. Individual commands only, no semicolon/pipe command bundles. Autonomous unfinished-plan repair is authorized under the existing policy.');
 await awaitMission('Parallel',item);assert.ok(item.peak>=2);assert.ok(item.kernelOverlap,'Two Worker PIDs must also be observed alive by the OS');assert.ok(item.peak<=2);assert.ok(new Set(item.intervals.map(interval=>interval.agent)).size>=2);
 const dependents=item.detail.tasks.filter(task=>task.state==='completed'&&task.dependencies.length>=2);assert.ok(dependents.length,'real Planner must create a dependent combined task');
 for(const dependent of dependents){const attempt=item.detail.attempts.find(attempt=>attempt.taskId===dependent.id);for(const id of dependent.dependencies){const predecessor=item.detail.attempts.find(attempt=>attempt.taskId===id&&attempt.status==='approved');assert.equal(run('git',['merge-base','--is-ancestor',item.scheduler.integrations.find(record=>record.task_id===id&&record.status==='published').candidate_sha,attempt.sourceBase],item.root).exit,0);}}
 });
 await test('C/D: isolated conflicting real edits cause a genuine Planner replan and repair',async()=>{
 const item=result.scenarios.Conflict;const objective='Update flags() to preserve base and return validation:true and logging:true, with independent tests for both and combined validation. An initial independent plan intentionally overlaps the same return expression, so preserve both outputs and autonomously repair the unfinished plan if Git cannot combine them. No force or side-selection resolution, no policy changes, no publication. Only src/** and tests/**. Use individual commands and python -m unittest discover -s tests -v; ASF owns commits/reviews/integration.';
 if(!item.conversation)await submit(item,objective,{objective,tasks:[task('A','Add validation flag','Modify only src/features.py and tests/test_validation.py. flags() must preserve base and add validation:true to the returned dict. Do not implement logging; a separate isolated task owns it. Test your addition.'),task('B','Add logging flag','Modify only src/features.py and tests/test_logging.py. flags() must preserve base and add logging:true to the returned dict. Do not implement validation; a separate isolated task owns it. Test your addition.')]});
 await awaitMission('Conflict',item);assert.ok(item.scheduler.integrations.some(record=>record.status==='conflict'));assert.ok(item.scheduler.revisions.length>=2);assert.ok(item.scheduler.revisions.some(revision=>revision.source==='replan'&&revision.plannerSessionId));assert.ok(item.detail.tasks.some(task=>task.state==='superseded'));
 });
 await test('E: failing compatibility evidence leads to bounded real-model repair',async()=>{
 const item=result.scenarios.Repair;const objective='Upgrade src/protocol.py encode(value) to return exactly a builtin dict {"value":str(value)} and adapt src/client.py request(value) to consume it while preserving uppercase string results. Add regression tests for both and run all unittest tests. Existing client tests must pass before integration. The initial plan separates producer migration from client adaptation and may expose a failing compatibility test; autonomously revise unfinished work and repair as necessary within the existing src/** and tests/** policy. Never claim a failed test passed, never broaden permissions, stage, commit, push or publish. Individual commands only.';
 if(!item.conversation)await submit(item,objective,{objective,tasks:[task('producer','Migrate producer interface','For this initial task, modify only src/protocol.py and tests/test_protocol.py. encode(value) must return exactly a builtin dict with value:str(value). Do not modify client.py in this task. Run all tests and report actual compatibility failures honestly. The mission coordinator may revise the unfinished plan if this isolated migration cannot be integrated.'),task('client','Adapt client after producer','Adapt src/client.py to consume the new dict and preserve uppercase string results, adding tests and validating the combined application.',['producer'])]});
 await awaitMission('Repair',item);assert.ok(item.detail.attempts.some(attempt=>attempt.status==='changes_requested'||attempt.status==='failed')||item.scheduler.integrations.some(record=>record.status==='conflict'),'actual failed/rejected evidence is required');assert.ok(item.scheduler.revisions.length>=2,'repair must replan unfinished work');
 });
 await test('Browser durable history, scoped CodeMirror roots, legacy entry and themes',async()=>{
 const item=result.scenarios.Parallel;await select(item);await panel().getByLabel('Conversation',{exact:true}).selectOption(item.conversation);await view('Progress');await page.reload();await page.getByLabel('Open registered project').waitFor();assert.equal((await get(item,`conversations/${item.conversation}/missions`))[0].status,'completed');await shot('restored-dark');
 await page.getByLabel('Theme',{exact:true}).selectOption('light');await shot('restored-light');
 const legacy=await context.newPage();await legacy.goto(base+'/');await legacy.getByRole('button',{name:'Agent Graph',exact:true}).waitFor();await legacy.close();
 });
 result.status='PASS';
}catch(error){result.status='FAIL';result.failure=String(error);console.error(error);if(page)await shot('failure').catch(()=>{});process.exitCode=1;}finally{result.finished=new Date().toISOString();await persist();await browser?.close();if(child.exitCode===null){child.kill();await new Promise(resolve=>child.once('exit',resolve));}}
