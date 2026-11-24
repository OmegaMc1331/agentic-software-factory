import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { createRequire, registerHooks } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";

const repository = process.cwd();
const dashboard = path.join(repository, "apps/dashboard");
assert.ok(process.argv[2], "Provide a fresh receipt path");
const output = path.resolve(repository, process.argv[2]);
const require = createRequire(path.join(dashboard, "package.json"));
const viteLibrary = pathToFileURL(require.resolve("vite"));
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const platform = process.platform;
const relevant = /(?:^|[\\/])node_modules[\\/](picomatch|tinyglobby|fdir)(?:[\\/]|$)/i;
const loaded = new Set();
const emitted = new Set();
const chunks = [];
const matcher = registerHooks({
  resolve(specifier, context, next) {
    const resolved = next(specifier, context);
    const dependency = resolved.url?.match(relevant)?.[1];
    if (dependency) loaded.add(dependency);
    return resolved;
  },
});
try {
  process.chdir(dashboard);
  const vite = await import(viteLibrary.href);
  const build = vite.build ?? vite.default?.build;
  assert.equal(typeof build, "function");
  await build({
    root: dashboard,
    logLevel: "error",
    build: { write: false },
    plugins: [
      {
        name: "asf-release-bundling-evidence",
        generateBundle(_options, bundle) {
          for (const chunk of Object.values(bundle)) {
            if (chunk.type !== "chunk") continue;
            chunks.push({ name: chunk.fileName, sha256: sha(chunk.code), modules: Object.keys(chunk.modules).length });
            for (const id of Object.keys(chunk.modules)) {
              const packageName = id.match(relevant)?.[1];
              if (packageName) emitted.add(packageName);
            }
          }
        },
      },
    ],
  });
} finally {
  process.chdir(repository);
  matcher.deregister();
}
assert.ok(chunks.length > 0, "Verify the actual emitted production chunks");
assert.ok(loaded.has("picomatch"), "Verify picomatch entered the Node build process");
assert.equal(emitted.size, 0, "Review unexpected matcher modules in browser chunks");
const record = {
  testId: "R1-PICOMATCH-PRODUCTION-BUNDLE-REACHABILITY",
  sourceCommit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  status: "PASS_ANALYSIS",
  timestamp: new Date().toISOString(),
  platform,
  node: process.version,
  lockSha256: sha(await readFile(path.join(dashboard, "package-lock.json"))),
  loadedInNodeBuild: [...loaded].sort(),
  emittedIntoBrowserChunks: [...emitted],
  chunks: chunks.sort((a, b) => a.name.localeCompare(b.name)),
  scope: "The normal Vite configuration emitted production modules in memory with an observer plugin. Node resolution and Rollup module identities cover actual tool loading and bundle membership. This is not a vulnerability test or release approval.",
};
await writeFile(output, JSON.stringify(record, null, 2) + "\n", { flag: "wx" });
console.log(JSON.stringify({ status: record.status, loadedInNodeBuild: record.loadedInNodeBuild, browserChunks: chunks.length }));
