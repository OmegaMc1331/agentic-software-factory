import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const output = process.env.ASF_VALIDATION_OUTPUT ?? path.join(root, ".factory/v2/p13/logs");
fs.mkdirSync(output, { recursive: true });
const mode = process.argv[2] ?? "focused";
const cargo = args => ["cargo", args, root];
const npm = args => [process.platform === "win32" ? "npm.cmd" : "npm", args, path.join(root, "apps/dashboard")];
const commands = {
  format: [cargo(["fmt", "--all", "--", "--check"])],
  refs: [cargo(["test","-p","factory-git","--test","source_control","unicode_worktree_aliases_require_canonical_ref_ownership","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-git","--test","source_control","branch_spelling_and_registered_worktree_ownership_are_enforced","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-git","--test","source_control","ref_case_aliases","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-api","--test","source_control","reserved_ref_case_aliases_cannot_bypass_authenticated_mutation_guards","-j","1","--","--test-threads=1"])],
  focused: [cargo(["test","-p","factory-api","--test","product","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-api","--lib","preferences::tests","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-db","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-memory","-j","1","--","--test-threads=1"]),cargo(["test","-p","factory-git","--test","source_control","-j","1","--","--test-threads=1"])],
  stress: [cargo(["test", "-p", "factory-core", "--test", "dynamic_scheduler", "bounded_stress", "-j", "1", "--", "--test-threads=1", "--nocapture"])],
  lifecycle: [cargo(["test", "-p", "factory-db", "settled_retry_tests", "-j", "1", "--", "--test-threads=1"]),cargo(["test", "-p", "factory-core", "--lib", "source_control::tests", "-j", "1", "--", "--test-threads=1"])],
  tools: [cargo(["test", "-p", "factory-ide", "--test", "services", "-j", "1", "--", "--ignored", "--test-threads=1", "--nocapture"])],
  git: [cargo(["test", "-p", "factory-git", "-j", "1", "--", "--test-threads=1"])],
  github: [cargo(["test", "-p", "factory-github", "-p", "factory-core", "--test", "github_delivery", "-p", "factory-api", "--test", "github_api", "-j", "1", "--", "--test-threads=1"])],
  rust: [cargo(["fmt", "--all", "--", "--check"]), cargo(["clippy", "--workspace", "--all-targets", "-j", "1", "--", "-D", "warnings"]), cargo(["test", "--workspace", "-j", "1", "--", "--test-threads=1"])],
  parallel: [cargo(["test", "--workspace", "-j", "1"])],
  api_parallel: [cargo(["test","-p","factory-api","--test","source_control","-j","1"])],
  clippy: [cargo(["clippy", "--workspace", "--all-targets", "-j", "1", "--", "-D", "warnings"])],
  frontend: [npm(["run", "format:check"]), npm(["run", "lint"]), npm(["run", "typecheck"]), npm(["test"]), npm(["run", "build"])],
  embedded: [cargo(["build", "-p", "factory-cli", "--features", "embedded-dashboard", "-j", "1"])],
};
if (!commands[mode]) throw new Error(`Unknown validation mode ${mode}`);
function fingerprint() {
  const paths = spawnSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "crates", "apps/dashboard", "Cargo.toml", "Cargo.lock", ".github/workflows"], { cwd: root, encoding: "utf8" }).stdout.split(/\r?\n/).filter(Boolean).filter(name => !name.includes("/dist/") && !name.includes("/node_modules/"));
  const hash = createHash("sha256");
  for (const name of [...new Set(paths)].sort()) { hash.update(name); hash.update(fs.readFileSync(path.join(root, name))); }
  return hash.digest("hex");
}
const source = fingerprint();
const result = { schema: 1, mode, startedAt: new Date().toISOString(), sourceCommit: spawnSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).stdout.trim(), sourceFingerprint: source, dirty: !!spawnSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" }).stdout.trim(), commands: [] };
const id = Date.now();
for (const [program, args, cwd] of commands[mode]) {
  const log = path.join(output, `${mode}-${id}-${result.commands.length + 1}.log`);
  const stream = fs.createWriteStream(log);
  const start = performance.now();
  const child = spawn(program, args, { cwd, windowsHide: true, shell: program.endsWith(".cmd"), env: { ...process.env, CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? path.join(root, "target"), CARGO_INCREMENTAL: "0", CARGO_PROFILE_DEV_DEBUG: "0", CARGO_PROFILE_TEST_DEBUG: "0" }, stdio: ["ignore", "pipe", "pipe"] });
  let text = "";
  for (const source of [child.stdout, child.stderr]) source.on("data", chunk => { stream.write(chunk); text += chunk.toString(); process.stdout.write(chunk); });
  const exit = await new Promise(resolve => { child.on("exit", resolve); child.on("error", error => { stream.write(String(error)); resolve(-1); }); });
  await new Promise(resolve => stream.end(resolve));
  const passed = [...text.matchAll(/test result: ok\. (\d+) passed/g)].reduce((n, match) => n + Number(match[1]), 0);
  result.commands.push({ command: [program, ...args], cwd, exit, durationMs: performance.now() - start, passedRustTests: passed, log: path.basename(log), sha256: createHash("sha256").update(fs.readFileSync(log)).digest("hex") });
  if (exit !== 0) break;
}
result.finalSourceFingerprint = fingerprint();
result.sourceUnchanged = result.sourceFingerprint === result.finalSourceFingerprint;
result.finishedAt = new Date().toISOString();
result.status = result.commands.length === commands[mode].length && result.commands.every(command => command.exit === 0) && result.sourceUnchanged ? "PASS" : "FAILED";
fs.writeFileSync(path.join(output, `${mode}-${id}.json`), JSON.stringify(result, null, 2));
fs.writeFileSync(path.join(output, `${mode}-results.json`), JSON.stringify(result, null, 2));
process.exitCode = result.status === "PASS" ? 0 : 1;
