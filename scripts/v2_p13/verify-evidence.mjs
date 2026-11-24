import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';

const root=process.cwd(),evidence=path.join(root,'docs/v2/evidence/p13');
const parse=bytes=>JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));
const read=async name=>parse(await fs.readFile(path.join(evidence,name)));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const manifest=await read('manifest.json');
assert.equal(manifest.schema,1);assert.equal(manifest.phase,'V2-P13');assert.equal(manifest.releaseStatus,'NO-GO');
assert.equal(manifest.limits.liveGitHubWrites,'NOT QUALIFIED — NO AUTHORIZED REMOTE MUTATION');
const names=new Set();
for(const reference of manifest.historicalReferences){const file=path.resolve(root,reference.file);assert.ok(file.startsWith(root+path.sep));assert.equal(hash(await fs.readFile(file)),reference.sha256,reference.file);}
for(const entry of manifest.files){
  assert.ok(!names.has(entry.file),'Duplicate manifest identity');names.add(entry.file);
  const file=path.resolve(evidence,entry.file);assert.ok(file.startsWith(evidence+path.sep),'Invalid manifest path');
  const info=await fs.lstat(file);assert.ok(info.isFile()&&!info.isSymbolicLink());
  assert.ok(!/\.(sqlite3?|db|exe)(?:-|$)/i.test(file),'Database/binary must remain local');
  const bytes=await fs.readFile(file);assert.equal(bytes.length,entry.bytes,entry.file);assert.equal(hash(bytes),entry.sha256,entry.file);
}
const current=manifest.currentRuntimeBuild;
assert.equal(current.buildValidation.status,'PASS');assert.equal(current.buildValidation.sourceUnchanged,true);
for(const name of ['clippy','format']){const result=await read('logs/'+name+'-results.json');assert.equal(result.status,'PASS');assert.equal(result.finalSourceFingerprint,current.sourceFingerprint);}
const frontend=await read('final-frontend-logs/frontend-results.json');assert.equal(frontend.status,'PASS');assert.equal(frontend.finalSourceFingerprint,current.sourceFingerprint);
const testLog=(await fs.readFile(path.join(evidence,'final-frontend-logs',frontend.commands[3].log),'utf8')).replace(/\x1b\[[0-9;]*m/g,'');
assert.match(testLog,/Tests\s+565 passed/);assert.equal(manifest.gates.frontendTests,565);
const serial=await read('final-rust-logs/rust-results.json');assert.equal(serial.commands.length,3);assert.ok(serial.commands.every(c=>c.exit===0));assert.equal(serial.commands.at(-1).passedRustTests,1072);
const normal=await read('logs/parallel-results.json');assert.equal(normal.commands[0].exit,0);assert.equal(normal.commands[0].passedRustTests,1072);assert.equal(normal.status,'FAILED');
const proof=await read('source-equivalence.json');assert.equal(proof.status,'RUST_AND_OTHER_INPUTS_IDENTICAL');assert.equal(proof.currentFingerprint,current.sourceFingerprint);assert.equal(proof.baselineFingerprint,normal.sourceFingerprint);assert.equal(proof.beforeKeyboardFingerprint,serial.sourceFingerprint);
const fresh=spawnSync(process.execPath,['scripts/v2_p13/source-equivalence.mjs'],{cwd:root,encoding:'utf8',windowsHide:true,env:{...process.env,ASF_P13_SOURCE_EVIDENCE:evidence,ASF_P13_SOURCE_PROOF_OUTPUT:path.join(root,'.factory/v2/p13/export-source-verification.json')}});
assert.equal(fresh.status,0,fresh.stderr||fresh.stdout);
const reconstructed=parse(await fs.readFile(path.join(root,'.factory/v2/p13/export-source-verification.json')));assert.equal(reconstructed.currentFingerprint,current.sourceFingerprint);
for(const driver of (await read('driver-identities.json')).drivers){const file=path.resolve(root,driver.file);assert.ok(file.startsWith(root+path.sep));assert.equal(hash(await fs.readFile(file)),driver.sha256,driver.file);}
for(const folder of ['perf-final-10','product-inspection-final-10','product-hunk-inspection-final-10','product-aligned-hunk-final-10']){const result=await read(folder+'/results.json');assert.equal(result.status,'PASS');assert.equal(result.binarySha256,current.sha256);assert.equal(result.remoteWrites,0);assert.equal(result.modelInvocations,0);assert.equal(result.cleanup.ownedServerExited,true);}
const browser=await read(manifest.gates.productBrowser.path);assert.equal(browser.status,'PASS');assert.equal(browser.binarySha256,current.sha256);assert.equal(browser.tests.length,11);assert.ok(browser.tests.every(t=>t.status==='PASS'));assert.equal(browser.accessibility.length,12);assert.ok(browser.accessibility.every(a=>a.violations.length===0));assert.deepEqual(browser.pageErrors,[]);assert.equal(browser.remoteWrites,0);
const regression=await read('browser-regressions-combined.json');assert.equal(regression.status,'PASS');assert.equal(regression.tests,104);
const model=await read('model-1/results.json'),a=model.projects.A;
assert.equal(model.status,'PASS');assert.equal(model.remoteWrites,0);assert.deepEqual(model.publicationRequests,[]);
assert.equal(a.detail.sessions.length,3);assert.deepEqual(a.detail.sessions.map(s=>s.role),['planner','worker','reviewer']);
for(const session of a.detail.sessions){assert.ok(session.nativeHeader.includes('model: gpt-6.1-sol'));assert.equal(session.outputMeta.processTreeCleanupConfirmed,true);assert.ok(!('stdout'in session)&&!('stderr'in session));}
assert.ok(a.detail.sessions.find(s=>s.role==='reviewer').nativeHeader.includes('sandbox: read-only'));
assert.equal(a.reviewer.reviewContext.capturedPatchSupplied,true);assert.equal(a.evidence.patchHashVerified,true);assert.equal(a.verification.exit,0);assert.equal(a.oracle.exit,0);
assert.match(a.verification.stderr,/Ran 10 tests/);assert.match(a.oracle.stdout,/500 independent/);assert.equal(a.integrations.length,1);
assert.ok(a.memoryUsage.every(s=>s.usage.some(u=>u.memories.some(m=>m.memoryId===a.memoryId&&m.revision===1))));
const inspected=await read('model-inspection-final-10/results.json');assert.equal(inspected.status,'PASS');assert.equal(inspected.binarySha256,current.sha256);assert.equal(inspected.modelInvocations,0);assert.equal(inspected.projects.A.integrationCommit,a.integrationCommit);
const stress=await read('stress-10000/verification.json');assert.equal(stress.records,10000);assert.equal(stress.revisions,30000);assert.equal(stress.exactFtsRows,10000);
for(const folder of ['runner-2-qualified','runner-4'])assert.equal((await read(folder+'/results.json')).status,'PASS');assert.equal((await read('runner-16/results.json')).status,'NOT_RUN_LOW_MEMORY_HEADROOM');
const review=await read('visual-review.json');assert.ok(review.images.length>=15);assert.equal(review.nativeAT,'NOT QUALIFIED');
for(const image of review.images){const bytes=await fs.readFile(path.join(evidence,image.file));assert.equal(hash(bytes),image.sha256);assert.ok(image.disposition);}
const lineage=await read('lineage.json');assert.ok(lineage.branches.every(b=>b.ancestorOfImplementationHead));
const descendant=spawnSync('git',['merge-base','--is-ancestor',manifest.implementationHead,'HEAD'],{cwd:root,windowsHide:true});assert.equal(descendant.status,0);
if(process.argv.includes('--git-index')){
  const entries=[...manifest.files,{file:'manifest.json',sha256:hash(await fs.readFile(path.join(evidence,'manifest.json')))}];
  for(const entry of entries)assert.ok(!/[\r\n]/.test(entry.file));
  const archive=spawnSync('git',['cat-file','--batch'],{cwd:root,input:entries.map(entry=>':docs/v2/evidence/p13/'+entry.file).join('\n')+'\n',windowsHide:true,maxBuffer:80*1024*1024});
  assert.equal(archive.status,0,archive.stderr.toString('utf8'));let offset=0;
  for(const entry of entries){const end=archive.stdout.indexOf(10,offset),header=archive.stdout.subarray(offset,end).toString('utf8').split(' ');assert.equal(header[1],'blob',entry.file);const size=Number(header[2]);assert.ok(Number.isSafeInteger(size)&&size>=0);const bytes=archive.stdout.subarray(end+1,end+1+size);assert.equal(hash(bytes),entry.sha256,'Staged Git blob '+entry.file);offset=end+1+size+1;}
  assert.equal(offset,archive.stdout.length);
}
console.log(JSON.stringify({status:'PASS',files:manifest.files.length,sourceFingerprint:current.sourceFingerprint,productJourneys:11,retainedBrowserJourneys:104,genuineRoles:3,gitIndexVerified:process.argv.includes('--git-index'),releaseStatus:manifest.releaseStatus}));
