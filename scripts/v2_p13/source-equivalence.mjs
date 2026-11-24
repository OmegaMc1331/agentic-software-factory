import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
const evidenceRoot=path.resolve(process.env.ASF_P13_SOURCE_EVIDENCE??'.factory/v2/p13');
const baseline=JSON.parse(fs.readFileSync(path.join(evidenceRoot,'run/build-5.json'),'utf8'));
const replacements={
 'apps/dashboard/src/v2/product/CommandPalette.tsx':'.factory/v2/p13/source-snapshots/CommandPalette.tsx.before',
 'apps/dashboard/src/v2/product/product.test.tsx':'.factory/v2/p13/source-snapshots/product.test.tsx.before',
 'apps/dashboard/src/v2/design-system/components/system.css':'.factory/v2/p13/source-snapshots/system.css.before',
 'apps/dashboard/src/v2/workspace/production/Views.tsx':'.factory/v2/p13/source-snapshots/Views.tsx.before',
 'apps/dashboard/src/v2/product/product.css':'.factory/v2/p13/source-snapshots/product.css.before',
 'apps/dashboard/src/v2/product/commands.ts':'.factory/v2/p13/source-snapshots/commands.ts.before',
 'apps/dashboard/src/v2/product/commands.test.ts':'.factory/v2/p13/source-snapshots/commands.test.ts.before',
 'apps/dashboard/src/v2/workspace/production/DockSurface.tsx':'.factory/v2/p13/source-snapshots/DockSurface.tsx.before',
 'apps/dashboard/src/v2/workspace/production/WorkspaceShell.tsx':'.factory/v2/p13/source-snapshots/WorkspaceShell.tsx.before',
 'apps/dashboard/src/v2/projects/ProjectsView.tsx':'.factory/v2/p13/source-snapshots/ProjectsView.tsx.before',
 'apps/dashboard/src/v2/projects/ProjectsView.test.tsx':'.factory/v2/p13/source-snapshots/ProjectsView.test.tsx.before',
};
for(const file of Object.keys(replacements))replacements[file]=path.join(evidenceRoot,'source-snapshots',path.basename(replacements[file]));
const listed=spawnSync('git',['ls-files','--cached','--others','--exclude-standard','crates','apps/dashboard','Cargo.toml','Cargo.lock','.github/workflows'],{encoding:'utf8',windowsHide:true});
if(listed.status!==0)throw Error('Unable to enumerate source inputs');
const files=listed.stdout.split(/\r?\n/).filter(Boolean).filter(p=>!p.includes('/dist/')&&!p.includes('/node_modules/'));
const old=createHash('sha256'),current=createHash('sha256');
const keyboard=createHash('sha256');
const keyboardFiles=['apps/dashboard/src/v2/product/CommandPalette.tsx','apps/dashboard/src/v2/product/product.test.tsx'];
for(const name of [...new Set(files)].sort()){old.update(name);old.update(fs.readFileSync(replacements[name]??name));current.update(name);current.update(fs.readFileSync(name));keyboard.update(name);keyboard.update(fs.readFileSync(keyboardFiles.includes(name)?replacements[name]:name));}
const evidence={schema:1,baselineBuild:5,baselineFingerprint:baseline.sourceFingerprint,reconstructedBaselineFingerprint:old.digest('hex'),currentFingerprint:current.digest('hex'),allowedDifferences:Object.entries(replacements).map(([file,before])=>({file,beforeSha256:createHash('sha256').update(fs.readFileSync(before)).digest('hex'),afterSha256:createHash('sha256').update(fs.readFileSync(file)).digest('hex')})),scope:'Exact baseline reconstruction permits only the listed frontend presentation files. All Rust, other frontend and build/workflow inputs must match.',checkedAt:new Date().toISOString()};
evidence.status=evidence.baselineFingerprint===evidence.reconstructedBaselineFingerprint?'RUST_AND_OTHER_INPUTS_IDENTICAL':'FAILED_UNEXPLAINED_SOURCE_DIFFERENCE';
evidence.beforeKeyboardFingerprint=keyboard.digest('hex');
evidence.beforeKeyboardExpected=JSON.parse(fs.readFileSync(path.join(evidenceRoot,'run/build-9.json'),'utf8')).sourceFingerprint;
evidence.keyboardBoundaryFiles=keyboardFiles;
if(evidence.beforeKeyboardFingerprint!==evidence.beforeKeyboardExpected)evidence.status='FAILED_UNEXPLAINED_KEYBOARD_BOUNDARY';
fs.writeFileSync(path.resolve(process.env.ASF_P13_SOURCE_PROOF_OUTPUT??'.factory/v2/p13/source-equivalence.json'),JSON.stringify(evidence,null,2)+'\n');console.log(JSON.stringify(evidence));
if(evidence.status!=='RUST_AND_OTHER_INPUTS_IDENTICAL')process.exitCode=1;
