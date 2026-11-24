import {spawn} from 'node:child_process';
import fs from 'node:fs';
const [workers,exitFile]=process.argv.slice(2);
if(!/^(?:[1-9]|1[0-6])$/.test(workers)||!exitFile)throw Error('Supply a bounded worker count and exit record');
const child=spawn(process.execPath,['node_modules/vitest/vitest.mjs','run',`--maxWorkers=${workers}`],{cwd:process.cwd(),windowsHide:true,stdio:'inherit'});
child.once('error',error=>{fs.writeFileSync(exitFile,JSON.stringify({code:null,error:String(error)}));process.exitCode=1;});
child.once('exit',(code,signal)=>{fs.writeFileSync(exitFile,JSON.stringify({code,signal,finishedAt:new Date().toISOString()}));process.exitCode=code??1;});
