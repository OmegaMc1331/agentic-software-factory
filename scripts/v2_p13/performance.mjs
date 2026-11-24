import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import net from "node:net";
import {spawn,spawnSync} from "node:child_process";
import {once} from "node:events";
import {createHash} from "node:crypto";

const root=process.cwd();
const binary=process.env.ASF_P13_BINARY;
assert.ok(binary,"Select an owned copied executable using ASF_P13_BINARY");
const output=process.env.ASF_P13_PERFORMANCE_OUTPUT;
assert.ok(output,"Select a fresh .factory/v2/p13 output directory");
const qualificationPhase=process.env.ASF_QUALIFICATION_PHASE??"p13";
assert.ok(["p13","p14"].includes(qualificationPhase));
assert.ok(path.resolve(output).startsWith(path.join(root,".factory","v2",qualificationPhase)+path.sep));
await fs.mkdir(output,{recursive:true});
const resultPath=path.join(output,"results.json");
await assert.rejects(fs.access(resultPath));
const build=JSON.parse(await fs.readFile(process.env.ASF_P13_BUILD_RECORD,"utf8"));
assert.equal(build.sha256,createHash("sha256").update(await fs.readFile(binary)).digest("hex"),"The copied binary must match its recorded build identity");
const result={schema:1,binaryBuild:build,mode:"controlled-real-API-Git-and-memory-performance-no-model",binarySha256:createHash("sha256").update(await fs.readFile(binary)).digest("hex"),driverCommit:spawnSync("git",["rev-parse","HEAD"],{encoding:"utf8",windowsHide:true}).stdout.trim(),startedAt:new Date().toISOString(),fixture:await fs.mkdtemp(path.join(os.tmpdir(),"asf-p13-perf-")),buildProfile:process.env.ASF_P13_BUILD_PROFILE??"unoptimized debug, debug symbols disabled",modelInvocations:0,remoteWrites:0,stressFixture:process.argv.includes("--stress"),projects:[],measurements:[],stages:[]};
const persist=()=>fs.writeFile(resultPath,JSON.stringify(result,null,2)+"\n");
const summary=xs=>{const s=[...xs].sort((a,b)=>a-b);return{samples:s.length,medianMs:s[Math.floor(s.length/2)],p95Ms:s[Math.min(s.length-1,Math.ceil(s.length*.95)-1)],minMs:s[0],maxMs:s.at(-1)}};
function run(program,args,cwd=root){const r=spawnSync(program,args,{cwd,encoding:"utf8",windowsHide:true,timeout:120000});assert.equal(r.status,0,`${program} failed: ${r.stderr}`);return r.stdout.trim();}
const host=()=>{
  if(process.platform!=="win32")return{platform:process.platform,cpus:os.cpus().map(c=>c.model),ramBytes:os.totalmem(),freeBytes:os.freemem(),loadAverage:os.loadavg()};
  const command="$o=Get-CimInstance Win32_OperatingSystem; $c=Get-CimInstance Win32_Processor; $d=Get-PhysicalDisk; $p=Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor | Where-Object Name -eq '_Total'; [pscustomobject]@{os=$o.Caption;build=$o.BuildNumber;cpu=@($c|Select-Object -ExpandProperty Name);logicalCpus=[Environment]::ProcessorCount;ramBytes=([long]$o.TotalVisibleMemorySize*1024);freeBytes=([long]$o.FreePhysicalMemory*1024);storage=@($d|Select-Object FriendlyName,MediaType,BusType,Size);cpuPercent=$p.PercentProcessorTime} | ConvertTo-Json -Depth 4 -Compress";
  return JSON.parse(run("powershell",["-NoProfile","-Command",command]));
};
let child,base,cookie,csrf;
async function api(url,method="GET",body){const r=await fetch(base+url,{method,headers:{origin:base,cookie,"content-type":"application/json","x-factory-csrf":csrf},...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(240000)});const t=await r.text();assert.ok(r.ok,`${method} ${url}: ${r.status} ${t.slice(0,400)}`);return t?JSON.parse(t):null;}
const scoped=(p,s,method="GET",body)=>api(`/api/projects/${p.id}/${s}`,method,body);
const sc=(p,s,body)=>scoped(p,`source-control/${s}`,body===undefined?"GET":"POST",body);
async function project(name,files){
  const folder=path.join(result.fixture,name);await fs.mkdir(folder,{recursive:true});
  await fs.writeFile(path.join(folder,".gitignore"),".factory/\n");
  for(let i=0;i<files;i++)await fs.writeFile(path.join(folder,`module_${i}.py`),`VALUE = ${i}\n`);
  for(const args of [["init","-q","-b","main"],["config","user.name","ASF P13 performance"],["config","user.email","p13@example.invalid"],["config","core.autocrlf","false"],["config","commit.gpgsign","false"],["add","."],["commit","-qm","controlled performance baseline"]])run("git",args,folder);
  run(binary,["init"],folder);
  return {name,root:folder,files};
}
async function timed(name,fn,count=5){const samples=[];let value;for(let i=0;i<count;i++){const began=performance.now();value=await fn(i);samples.push(performance.now()-began);}result.measurements.push({name,...summary(samples),rawMs:samples});await persist();return value;}
async function memory(p){
  let current=0;
  const counts=(process.env.ASF_P13_MEMORY_COUNTS??"100,1000").split(",").map(Number);
  assert.ok(counts.every((n,i)=>Number.isInteger(n)&&n>0&&n<=10000&&(i===0||n>counts[i-1])));
  for(const count of counts){
    const creates=[],updates=[];const began=performance.now();let first;
    for(;current<count;current++){
      const i=current;const draft={category:"coding_conventions",title:`Adapter fixture component ${i}`,content:process.argv.includes("--stress")?`Component_${i} uses a repository adapter. Regression tests preserve compatible retries. Synthetic fixture, not project knowledge.`:`Component_${i} uses a repository adapter to validate transactional boundary conditions. Changes preserve backwards compatible inputs and verify repeated requests without duplicating side effects. This generated fact measures local retrieval and persistence, not model quality.`,subject:`component_${i}`,scope:`component_${i}`,status:"active",sources:[{projectId:p.id,reference:{kind:"manual",note:"Explicit synthetic performance fixture"}}]};
      let at=performance.now();let m=await scoped(p,"memory","POST",{memory:draft,reason:"Controlled synthetic performance data"});creates.push(performance.now()-at);
      for(let revision=2;revision<=3;revision++){at=performance.now();m=await scoped(p,`memory/${m.id}`,"PATCH",{memory:{...draft,content:draft.content+` Regression revision ${revision}.`},expectedRevision:m.revision,reason:"Controlled synthetic revision"});updates.push(performance.now()-at);}
      first??=m;
      if((i+1)%100===0)process.stdout.write(JSON.stringify({stage:"memory",records:i+1,target:count})+"\n");
    }
    result.stages.push({kind:"memory",records:count,createdThisStage:creates.length,loadMs:performance.now()-began,create:summary(creates),update:summary(updates),revisionSample:first?.revision});await persist();
    await timed(`memory.${count}.search`,()=>scoped(p,"memory/search","POST",{query:"repository adapter",limit:20}),10);
    await timed(`memory.${count}.retrieve`,()=>scoped(p,"memory/retrieve","POST",{query:"repository adapter"}),10);
    await timed(`memory.${count}.rebuild`,()=>scoped(p,"memory/index/rebuild","POST",{}),3);
  }
}
async function git(p){
  await timed(`git.${p.files}.cleanStatus`,()=>sc(p,`status?rootId=${p.rootId}`));
  for(let i=0;i<Math.min(200,p.files);i++)await fs.writeFile(path.join(p.root,`module_${i}.py`),`VALUE = ${i+100000}\n`);
  await timed(`git.${p.files}.changedStatus`,()=>sc(p,`status?rootId=${p.rootId}`));
  await timed(`git.${p.files}.diff`,()=>sc(p,"diff",{rootId:p.rootId,diff:{path:"module_0.py",comparison:"working_index",base:null,target:null}}));
  let status=(await sc(p,`status?rootId=${p.rootId}`)).status;
  await timed(`git.${p.files}.fileStage`,()=>sc(p,"index",{rootId:p.rootId,key:crypto.randomUUID(),request:{expectedGeneration:status.generation,paths:["module_0.py"],unstage:false,hunk:null,diffVersion:null,trustPrograms:false}}),1);
  status=(await sc(p,`status?rootId=${p.rootId}`)).status;
  await timed(`git.${p.files}.commit`,()=>sc(p,"commit",{rootId:p.rootId,key:crypto.randomUUID(),expectedGeneration:status.generation,message:"Controlled exact-index sample",trustPrograms:false}),1);
  assert.equal(run("git",["show","HEAD:module_0.py"],p.root),"VALUE = 100000");
  assert.ok(run("git",["diff","--name-only"],p.root).includes("module_1.py"));
}
try {
  result.hostBefore=host();
  result.projects.push(await project("Memory",1));
  for(const files of (process.env.ASF_P13_GIT_COUNTS??"100,1000,6000").split(",").map(Number)) {assert.ok(files>1&&files<=10000);result.projects.push(await project(`Git${files}`,files));}
  const port=await new Promise(resolve=>{const server=net.createServer();server.listen(0,"127.0.0.1",()=>{const port=server.address().port;server.close(()=>resolve(port));});});
  base=`http://127.0.0.1:${port}`;
  const log=await fs.open(path.join(output,"server.log"),"w");
  child=spawn(binary,["start","--no-browser","--port",String(port),"--registry",path.join(result.fixture,"registry.sqlite3"),"--allow-project-root",result.fixture],{cwd:result.projects[0].root,windowsHide:true,stdio:["ignore",log.fd,log.fd]});await log.close();
  result.serverPid=child.pid;await persist();
  for(let attempt=0;attempt<120;attempt++){try{if((await fetch(base+"/api/health")).ok)break;}catch{}await new Promise(r=>setTimeout(r,250));}
  const auth=await fetch(base+"/api/auth/session",{method:"POST",headers:{origin:base,"content-type":"application/json"},body:"{}"});assert.ok(auth.ok);cookie=auth.headers.getSetCookie().map(s=>s.split(";")[0]).join("; ");csrf=(await auth.json()).csrfToken;
  const entries=await api("/api/projects");
  for(const p of result.projects){let entry=entries.find(e=>path.resolve(e.canonical_root.replace(/^\\\\\?\\/,"")).toLowerCase()===p.root.toLowerCase());entry??=await api("/api/projects/register","POST",{path:p.root,trust_project:true});p.id=entry.project_id;await scoped(p,"open","POST");p.rootId=(await scoped(p,"files/roots")).roots.find(r=>r.kind==="repository").id;}
  if(!process.argv.includes("--git-only"))await memory(result.projects[0]);
  if(!process.argv.includes("--memory-only"))for(const p of result.projects.slice(1))await git(p);
  result.databaseFiles=await Promise.all(result.projects.map(async p=>{const base=path.join(p.root,".factory/db.sqlite3");const size=async name=>{try{return(await fs.stat(name)).size;}catch(e){if(e.code==="ENOENT")return 0;throw e;}};return{project:p.id,name:p.name,databaseBytes:await size(base),walBytes:await size(base+"-wal")};}));
  if(process.platform==="win32")result.serverMemory=JSON.parse(run("powershell",["-NoProfile","-Command",`Get-Process -Id ${Number(child.pid)} | Select-Object Id,WorkingSet64,PrivateMemorySize64 | ConvertTo-Json -Compress`]));
  result.memoryMeasurementScope="One owned server-process working-set/private-commit sample after operations; not a peak or whole-host measurement";
  result.hostAfter=host();result.status="PASS";
}catch(error){result.status="FAILED";result.error=String(error);process.exitCode=1;}
finally {
  if(child&&child.exitCode===null){const exit=once(child,"exit");child.kill();let timer;try{await Promise.race([exit,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error("Owned test server termination was not observed")),10000);})]);}finally{clearTimeout(timer);}}
  result.cleanup={ownedServerExited:child?child.exitCode!==null||child.signalCode!==null:true};result.finishedAt=new Date().toISOString();await persist();process.stdout.write(JSON.stringify({status:result.status,output,error:result.error})+"\n");
}
