import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import net from 'node:net';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import {spawn,spawnSync} from 'node:child_process';

const root=process.cwd(),binary=process.env.ASF_P13_BINARY,output=process.env.ASF_P13_OUTPUT;
assert.ok(binary&&output);
const source=process.env.ASF_P13_BROWSER_SOURCE??'.factory/v2/p13/browser-final-10-keyboard/results.json';
const sourceBytes=await fs.readFile(source),prior=JSON.parse(sourceBytes);assert.equal(prior.status,'PASS');
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
assert.equal(hash(await fs.readFile(binary)),prior.binarySha256);
await fs.mkdir(path.join(output,'screenshots'),{recursive:true});await assert.rejects(fs.access(path.join(output,'results.json')));
const require=createRequire(path.join(root,'apps/dashboard/package.json')),{chromium}=require('playwright');
const result={schema:1,mode:'model-free-read-only-Git-inspection-of-completed-product-fixture',startedAt:new Date().toISOString(),binarySha256:prior.binarySha256,sourceSha256:hash(sourceBytes),modelInvocations:0,remoteWrites:0,gitMutations:[],screenshots:[],pageErrors:[]};
const a=prior.projects.A;
function git(args){const r=spawnSync('git',args,{cwd:a.root,encoding:'utf8',windowsHide:true});assert.equal(r.status,0,r.stderr);return r.stdout.trim();}
async function until(action,label){const deadline=Date.now()+45000;while(Date.now()<deadline){const value=await action();if(value)return value;await new Promise(resolve=>setTimeout(resolve,200));}throw Error('Timed out: '+label);}
const port=await new Promise(resolve=>{const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const port=s.address().port;s.close(()=>resolve(port));});});
const base=`http://127.0.0.1:${port}`,log=await fs.open(path.join(output,'server.log'),'w');
const child=spawn(binary,['start','--no-browser','--no-default-project','--port',String(port),'--registry',path.join(prior.fixture,'registry.sqlite3'),'--allow-project-root',prior.fixture],{cwd:a.root,windowsHide:true,stdio:['ignore',log.fd,log.fd]});await log.close();
let browser;
try{
  await until(async()=>{try{return(await fetch(base+'/api/health')).ok;}catch{return false;}},'owned server');
  result.headBefore=git(['rev-parse','HEAD']);result.indexBefore=git(['diff','--cached','--binary']);result.workingBefore=git(['diff','--binary']);assert.ok(result.workingBefore.includes('VALUE_30 = 3000'));
  browser=await chromium.launch({channel:'msedge',headless:true,args:['--disable-gpu','--disable-extensions','--disable-background-networking']});
  const context=await browser.newContext({viewport:{width:1600,height:900},reducedMotion:'reduce'}),page=await context.newPage();
  page.on('pageerror',error=>result.pageErrors.push(String(error)));
  page.on('request',request=>{if(request.method()!=='GET'&&/\/source-control\/(?:index|commit|branches|delivery)(?:\?|$)/.test(new URL(request.url()).pathname))result.gitMutations.push({method:request.method(),path:new URL(request.url()).pathname});});
  await page.goto(base+'/workspace.html');await page.getByLabel('Open registered project').selectOption(a.id);await until(()=>page.locator('#project-'+a.id).getAttribute('aria-selected').then(value=>value==='true'),'owning project');
  await page.getByRole('button',{name:'Views',exact:true}).click();await page.getByRole('menuitem',{name:'Source Control',exact:true}).click();
  const panel=page.locator('.asf-source:visible').last();await panel.getByRole('button',{name:'Refresh repository',exact:true}).click();await panel.locator('.sc-file-list button[title]').filter({hasText:'sample.py'}).first().click();
  const target=panel.getByLabel('Git target version',{exact:true});await target.waitFor();assert.ok((await target.innerText()).includes('VALUE_30 = 3000'));assert.ok((await panel.getByLabel('Git base version',{exact:true}).innerText()).includes('VALUE_30 = 30'));await panel.getByRole('button',{name:'Stage hunk 1',exact:true}).waitFor();
  await panel.getByRole('button',{name:/^Hunk 1:/}).click();await target.locator('.cm-line').filter({hasText:'VALUE_30 = 3000'}).scrollIntoViewIfNeeded();await panel.getByLabel('Git base version',{exact:true}).locator('.cm-line').filter({hasText:/VALUE_30 = 30$/}).scrollIntoViewIfNeeded();
  const file='screenshots/actual-remaining-hunk.png';await page.screenshot({path:path.join(output,file)});result.screenshots.push({file,sha256:hash(await fs.readFile(path.join(output,file)))});
  if(!process.argv.includes('--image-only')){
  const cdp=await context.newCDPSession(page);
  const sample=async()=>{await cdp.send('HeapProfiler.collectGarbage');return{...await cdp.send('Runtime.getHeapUsage'),domNodes:await page.evaluate(()=>document.getElementsByTagName('*').length)};};
  result.resourceDiagnostic={method:'Six real Memory open/close/unmount cycles. Explicit CDP garbage collection precedes each JavaScript heap sample; this is a separate retained-heap diagnostic, not ordinary UI latency or OS memory.',before:await sample(),cycles:[]};
  const command=async title=>{await page.getByRole('button',{name:'Commands',exact:true}).click();await page.getByLabel('Search commands',{exact:true}).fill(title);await page.getByLabel('Search commands',{exact:true}).press('Enter');};
  for(let cycle=0;cycle<6;cycle++){
    await command('Open Memory');await page.locator('.asf-memory:visible').last().waitFor();await command('Close current presentation tab');await until(async()=>(await page.locator('.asf-memory:visible').count())===0,'actual Memory unmount');await until(async()=>(await page.locator('.ws-status').innerText()).includes('Workspace saved'),'durable presentation save');result.resourceDiagnostic.cycles.push(await sample());
  }
  await cdp.detach();
  }
  assert.equal(git(['rev-parse','HEAD']),result.headBefore);assert.equal(git(['diff','--cached','--binary']),result.indexBefore);assert.equal(git(['diff','--binary']),result.workingBefore);assert.deepEqual(result.gitMutations,[]);assert.deepEqual(result.pageErrors,[]);result.status='PASS';
}catch(error){result.status='FAILED';result.error=String(error.stack??error);process.exitCode=1;}
finally{await browser?.close();if(child.exitCode===null){const exited=new Promise(resolve=>child.once('exit',resolve));child.kill();await exited;}result.cleanup={ownedServerExited:child.exitCode!==null||child.signalCode!==null};result.finishedAt=new Date().toISOString();await fs.writeFile(path.join(output,'results.json'),JSON.stringify(result,null,2)+'\n');console.log(JSON.stringify({status:result.status,output,error:result.error}));}
