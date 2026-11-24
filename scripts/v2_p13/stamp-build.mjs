import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
const name=process.argv[2];
if(!/^[1-9][0-9]*$/.test(name??''))throw Error('Supply a new numeric build identity');
const output=path.resolve('.factory/v2/p13/run');
const validation=JSON.parse(await fs.readFile('.factory/v2/p13/logs/embedded-results.json','utf8'));
if(validation.status!=='PASS'||!validation.sourceUnchanged)throw Error('Require a completed unchanged-source embedded build');
const files=spawnSync('git',['ls-files','--cached','--others','--exclude-standard','crates','apps/dashboard','Cargo.toml','Cargo.lock','.github/workflows'],{encoding:'utf8',windowsHide:true}).stdout.split(/\r?\n/).filter(Boolean).filter(p=>!p.includes('/dist/')&&!p.includes('/node_modules/'));
const source=createHash('sha256');for(const p of [...new Set(files)].sort()){source.update(p);source.update(await fs.readFile(p));}
if(source.digest('hex')!==validation.finalSourceFingerprint)throw Error('Source changed after the embedded build');
await fs.mkdir(output,{recursive:true});
const target=path.join(output,`factory-p13-${name}.exe`);await assertMissing(target);
const bytes=await fs.readFile(path.join(process.env.CARGO_TARGET_DIR??path.join(root, "target"),'debug','factory.exe'));
await fs.writeFile(target,bytes,{flag:'wx'});
const metadata={schema:1,path:target,sha256:createHash('sha256').update(bytes).digest('hex'),sourceCommit:validation.sourceCommit,sourceFingerprint:validation.sourceFingerprint,buildProfile:'unoptimized debug, debug symbols disabled',builtAt:new Date().toISOString(),buildValidation:validation};
await fs.writeFile(path.join(output,`build-${name}.json`),JSON.stringify(metadata,null,2)+'\n',{flag:'wx'});
console.log(JSON.stringify({binary:target,sha256:metadata.sha256,sourceFingerprint:metadata.sourceFingerprint}));
async function assertMissing(file){try{await fs.access(file);}catch(e){if(e.code==='ENOENT')return;throw e;}throw Error('Preserve existing build '+file);}
