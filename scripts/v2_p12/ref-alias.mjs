import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import {spawn,spawnSync} from "node:child_process";
import {createHash,randomUUID} from "node:crypto";

const binary=process.env.ASF_P12_BINARY;
assert.ok(binary,"Select an owned copied test executable");
const output=path.resolve(process.env.ASF_P12_REF_OUTPUT??".factory/v2/p12/ref-alias-fixed.json");
try{await fs.access(output);throw Error("Preserve existing evidence; choose another output");}catch(error){if(error.code!=="ENOENT")throw error;}
const parent=await fs.mkdtemp(path.join(os.tmpdir(),"asf-p12-ref-api-"));
const root=path.join(parent,"project");await fs.mkdir(root);
const hash=bytes=>createHash("sha256").update(bytes).digest("hex");
const result={schema:1,startedAt:new Date().toISOString(),binarySha256:hash(await fs.readFile(binary)),scriptSha256:hash(await fs.readFile(new URL(import.meta.url))),fixture:root,modelInvocations:0,remoteWrites:0,requests:[]};
function run(program,args){const r=spawnSync(program,args,{cwd:root,encoding:"utf8",windowsHide:true});assert.equal(r.status,0,r.stderr);return r.stdout.trim();}
await fs.writeFile(path.join(root,".gitignore"),".factory/\n");await fs.writeFile(path.join(root,"sample.txt"),"baseline\n");
for(const args of [["init","-q","-b","main"],["config","user.name","ASF P12 Ref Safety"],["config","user.email","p12@example.invalid"],["config","commit.gpgsign","false"],["add","."],["commit","-qm","owned baseline"],["branch","factory/reserved"]])run("git",args);
run(binary,["init"]);
result.initialHead=run("git",["rev-parse","HEAD"]);
const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,"127.0.0.1",()=>{const p=s.address().port;s.close(()=>resolve(p));});});
const base=`http://127.0.0.1:${port}`;
const child=spawn(binary,["start","--no-browser","--port",String(port),"--registry",path.join(parent,"registry.sqlite3"),"--allow-project-root",parent],{cwd:root,stdio:"ignore",windowsHide:true});result.ownedServerPid=child.pid;
let cookie,csrf;
async function api(url,method="GET",body){const started=performance.now();const r=await fetch(base+url,{method,headers:{Origin:base,...(cookie?{Cookie:cookie,"X-Factory-CSRF":csrf}:{}),...(body?{"Content-Type":"application/json"}:{})},...(body?{body:JSON.stringify(body)}:{})});const value=await r.json();result.requests.push({path:url,method,status:r.status,durationMs:performance.now()-started});return {status:r.status,value};}
try{
 const deadline=Date.now()+30000;while(true){try{if((await fetch(base+"/api/health")).ok)break;}catch{}assert.ok(Date.now()<deadline,"Owned server startup");await new Promise(resolve=>setTimeout(resolve,100));}
 const auth=await fetch(base+"/api/auth/session",{method:"POST",headers:{Origin:base,"Content-Type":"application/json"},body:"{}"});assert.equal(auth.status,200);cookie=auth.headers.getSetCookie().map(v=>v.split(";")[0]).join("; ");csrf=(await auth.json()).csrfToken;
 const projects=await api("/api/projects");assert.equal(projects.status,200);const project=projects.value.find(p=>{const prefix=String.fromCharCode(92,92,63,92);const canonical=p.canonical_root.startsWith(prefix)?p.canonical_root.slice(4):p.canonical_root;return path.resolve(canonical).toLowerCase()===root.toLowerCase();});assert.ok(project,"Exact owned project identity");result.projectId=project.project_id;
 assert.equal((await api(`/api/projects/${project.project_id}/open`,"POST",{})).status,200);
 const prefix=`/api/projects/${project.project_id}`;const roots=(await api(prefix+"/files/roots")).value;const primary=roots.roots.find(r=>r.kind==="repository");assert.ok(primary&&!primary.read_only);
 async function branch(name,create){const current=await api(prefix+`/source-control/status?rootId=${primary.id}`);assert.equal(current.status,200);return api(prefix+"/source-control/branches","POST",{rootId:primary.id,key:randomUUID(),expectedGeneration:current.value.status.generation,name,create,trustPrograms:false,dirtyBuffers:false});}
 result.safeCreation=await branch("feature/safe",true);assert.equal(result.safeCreation.status,200);
 result.safeReturn=await branch("main",false);assert.equal(result.safeReturn.status,200);
 result.caseAlias=await branch("Factory/reserved",false);result.finalBranch=run("git",["branch","--show-current"]);result.finalHead=run("git",["rev-parse","HEAD"]);assert.equal(result.finalHead,result.initialHead);
 if(process.argv.includes("--expect-vulnerable")){assert.equal(result.caseAlias.status,200);assert.equal(result.finalBranch,"Factory/reserved");result.status="VULNERABILITY_REPRODUCED";}else{assert.equal(result.caseAlias.status,400,JSON.stringify(result.caseAlias));assert.equal(result.finalBranch,"main");run("git",["worktree","add","-b","feature/owned",path.join(parent,"owned")]);
 result.ownedAlias=await branch("Feature/owned",false);assert.equal(result.ownedAlias.status,400);
 result.ownedExact=await branch("feature/owned",false);assert.equal(result.ownedExact.status,403);
 result.ownedCreationAlias=await branch("FEATURE/owned",true);assert.equal(result.ownedCreationAlias.status,400);
 assert.equal(run("git",["branch","--show-current"]),"main");
run("git",["worktree","add","-b","f\u00e9ature/owned",path.join(parent,"unicode-owned")]);
 run("git",["-C",path.join(parent,"unicode-owned"),"symbolic-ref","HEAD","refs/heads/f\u00c9ature/owned"]);
 result.unicodeOwned=await branch("f\u00e9ature/owned",false);
 result.finalBranch=run("git",["branch","--show-current"]);result.finalHead=run("git",["rev-parse","HEAD"]);assert.equal(result.finalHead,result.initialHead);
 if(process.argv.includes("--expect-unicode-vulnerable")){assert.equal(result.unicodeOwned.status,200);assert.equal(result.finalBranch,"f\u00e9ature/owned");result.status="UNICODE_OWNERSHIP_BYPASS_REPRODUCED";}
 else {assert.equal(result.unicodeOwned.status,403);assert.equal(result.finalBranch,"main");result.status="GUARD_ENFORCED";}}
}catch(error){result.status="FAILED";result.error=String(error.stack??error);process.exitCode=1;}
finally{if(child.exitCode===null){const exited=new Promise(resolve=>child.once("exit",resolve));child.kill();await exited;}result.cleanup={ownedServerExited:child.exitCode!==null||child.signalCode!==null};result.finishedAt=new Date().toISOString();await fs.writeFile(output,JSON.stringify(result,null,2)+"\n");process.stdout.write(JSON.stringify({status:result.status,output,error:result.error}));}
