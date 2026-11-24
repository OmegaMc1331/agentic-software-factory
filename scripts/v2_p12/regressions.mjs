import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const binary = process.env.ASF_P12_BINARY;
if (!binary || !fs.existsSync(binary)) throw Error("Set ASF_P12_BINARY to the copied qualified executable");
const output = path.resolve(process.env.ASF_P12_REGRESSION_OUTPUT ?? path.join(root, ".factory/v2/p12/browser-regressions"));
const phases = process.argv.slice(2).length ? process.argv.slice(2) : ["03", "04", "05", "06", "07"];
if (phases.some(phase => !["03", "04", "05", "06", "07"].includes(phase))) throw Error("Unknown retained browser phase");
fs.mkdirSync(output, { recursive: true });
const results = { schema: 1, binary, binarySha256: createHash("sha256").update(fs.readFileSync(binary)).digest("hex"), startedAt: new Date().toISOString(), phases: [], modelInvocations: 0 };
for (const phase of phases) {
  const directory = path.join(output, `p${phase}`);
  const resultPath = path.join(directory, phase === "05" ? "editor-browser-results.json" : "browser-results.json");
  if (fs.existsSync(resultPath)) throw Error(`Preserve existing evidence in ${directory}`);
  fs.mkdirSync(directory, { recursive: true });
  const log = fs.createWriteStream(path.join(directory, "runner.log"));
  const child = spawn(process.execPath, [`scripts/v2_p${phase}/browser.mjs`], {
    cwd: root,
    windowsHide: true,
    env: { ...process.env, [`ASF_P${phase}_BINARY`]: binary, [`ASF_P${phase}_OUTPUT_DIR`]: directory, ASF_P07_REAL_MODE: "controlled", ASF_IDE_TEST_TOOLS: process.env.ASF_IDE_TEST_TOOLS ?? path.join(root, ".factory/v2/p11/tools/profiles.json") },
    stdio: ["ignore", "pipe", "pipe"],
  });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log.write(chunk); process.stdout.write(chunk); });
  let exit = await new Promise(resolve => { child.once("exit", resolve); child.once("error", error => { log.write(String(error)); resolve(-1); }); });
  await new Promise(resolve => log.end(resolve));
  const record = { phase, exit, resultPath };
  if (phase === "07" && exit === 0) {
    const recoveryLog=fs.createWriteStream(path.join(directory,"recovery-runner.log"));
    const recoveryChild=spawn(process.execPath,["scripts/v2_p07/recovery-browser.mjs"],{cwd:root,windowsHide:true,env:{...process.env,ASF_P07_BINARY:binary,ASF_P07_OUTPUT_DIR:directory,ASF_P07_REAL_MODE:"controlled"},stdio:["ignore","pipe","pipe"]});
    for(const stream of [recoveryChild.stdout,recoveryChild.stderr])stream.on("data",chunk=>{recoveryLog.write(chunk);process.stdout.write(chunk);});
    exit=await new Promise(resolve=>{recoveryChild.once("exit",resolve);recoveryChild.once("error",error=>{recoveryLog.write(String(error));resolve(-1);});});
    await new Promise(resolve=>recoveryLog.end(resolve));record.recoveryExit=exit;record.exit=exit;
  }
  if (fs.existsSync(record.resultPath)) {
    const data = JSON.parse(fs.readFileSync(record.resultPath, "utf8"));
    record.tests = data.tests?.length;
    record.failure = data.failure;
    record.sha256 = createHash("sha256").update(fs.readFileSync(record.resultPath)).digest("hex");
  }
  const recovery = path.join(directory, "recovery-browser-results.json");
  if (phase === "07" && fs.existsSync(recovery)) {
    const data = JSON.parse(fs.readFileSync(recovery, "utf8"));
    record.recoveryTests = data.tests?.length;
    record.recoverySha256 = createHash("sha256").update(fs.readFileSync(recovery)).digest("hex");
  }
  results.phases.push(record);
  fs.writeFileSync(path.join(output, "summary.json"), JSON.stringify(results, null, 2));
  if (exit !== 0) { process.exitCode = 1; break; }
}
results.finishedAt = new Date().toISOString();
results.status = results.phases.length === phases.length && results.phases.every(phase => phase.exit === 0) ? "PASS" : "FAILED";
fs.writeFileSync(path.join(output, "summary.json"), JSON.stringify(results, null, 2));
