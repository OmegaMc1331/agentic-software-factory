import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT = fileURLToPath(new URL("release-checksums.mjs", import.meta.url));
const INVENTORY_PATH = fileURLToPath(
  new URL("release-artifacts.txt", import.meta.url),
);
const WORKFLOW_PATH = fileURLToPath(
  new URL("../.github/workflows/release.yml", import.meta.url),
);
const ARCHIVES = [
  "factory-linux-x86_64.tar.gz",
  "factory-macos-aarch64.tar.gz",
  "factory-macos-x86_64.tar.gz",
  "factory-windows-x86_64.zip",
];
const ZERO_DIGEST = "0".repeat(64);

function workspace(t) {
  const directory = mkdtempSync(join(tmpdir(), "factory-release-checksums-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  return directory;
}

function run(args, cwd) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    cwd,
    encoding: "utf8",
  });
}

function expectOk(result) {
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

function expectFailure(result, pattern) {
  assert.notEqual(
    result.status,
    0,
    `expected failure but command succeeded: ${result.stdout}`,
  );
  if (pattern) {
    assert.match(result.stderr, pattern);
  }
}

function writeArchives(directory, names = ARCHIVES) {
  for (const [index, name] of names.entries()) {
    writeFileSync(join(directory, name), `fixture archive ${index}: ${name}\n`);
  }
}

function generateAll(directory, names = ARCHIVES) {
  writeArchives(directory, names);
  for (const name of names) {
    expectOk(run(["generate", name], directory));
  }
}

function manifestFor(directory, names = ARCHIVES, output = "SHA256SUMS.txt") {
  expectOk(run(["manifest", output, ...names], directory));
  return join(directory, output);
}

function linesOf(file) {
  return readFileSync(file, "utf8").trimEnd().split("\n");
}

function readText(file) {
  return readFileSync(file, "utf8").replace(/\r\n/g, "\n");
}

function digestFrom(directory, name) {
  return readFileSync(join(directory, `${name}.sha256`), "utf8").split("  ")[0];
}

function writeChecksum(directory, name, digest, referenced = name) {
  writeFileSync(
    join(directory, `${name}.sha256`),
    `${digest}  ${referenced}\n`,
  );
}

function writeManifest(directory, entries, output = "SHA256SUMS.txt") {
  writeFileSync(
    join(directory, output),
    `${entries.map(([digest, name]) => `${digest}  ${name}`).join("\n")}\n`,
  );
}

function commandAvailable(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  return !result.error && result.status === 0;
}

test("valid Linux archive passes per-archive verification", (t) => {
  const directory = workspace(t);
  generateAll(directory, ["factory-linux-x86_64.tar.gz"]);
  expectOk(run(["verify", "factory-linux-x86_64.tar.gz"], directory));
});

test("valid Windows archive passes per-archive verification", (t) => {
  const directory = workspace(t);
  generateAll(directory, ["factory-windows-x86_64.zip"]);
  expectOk(run(["verify", "factory-windows-x86_64.zip"], directory));
});

test("valid macOS Apple Silicon archive passes per-archive verification", (t) => {
  const directory = workspace(t);
  generateAll(directory, ["factory-macos-aarch64.tar.gz"]);
  expectOk(run(["verify", "factory-macos-aarch64.tar.gz"], directory));
});

test("valid macOS Intel archive passes per-archive verification", (t) => {
  const directory = workspace(t);
  generateAll(directory, ["factory-macos-x86_64.tar.gz"]);
  expectOk(run(["verify", "factory-macos-x86_64.tar.gz"], directory));
});

test("all four archives pass together", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  expectOk(run(["verify-manifest", manifest, ...ARCHIVES], directory));
});

test("per-archive checksum verification succeeds for every archive", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  for (const archive of ARCHIVES) {
    expectOk(run(["verify", archive], directory));
  }
});

test("generated checksum files use the canonical one-line format", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  const text = readFileSync(join(directory, `${name}.sha256`), "utf8");
  assert.match(text, /^[0-9a-f]{64}  factory-linux-x86_64\.tar\.gz\n$/);
  assert.ok(text.endsWith("\n"));
  assert.equal(text.split("\n").length, 2);
});

test("consolidated checksum verification succeeds", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  expectOk(run(["verify-manifest", manifest, ...ARCHIVES], directory));
});

test("verify-release checks the exact downloaded inventory", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  expectOk(run(["verify-release", directory], directory));
});

test("artifact inventory accepts Windows CRLF line endings", (t) => {
  const directory = workspace(t);
  const release = join(directory, "release");
  mkdirSync(release);
  generateAll(release);
  const inventory = join(directory, "inventory.txt");
  writeFileSync(inventory, `${ARCHIVES.join("\r\n")}\r\n`);
  expectOk(run(["verify-release", release, inventory], directory));
});

test("standard sha256sum -c works from a flat directory", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  manifestFor(directory);
  const hasSha256Sum = commandAvailable("sha256sum", ["--version"]);
  const hasShasum = commandAvailable("shasum", ["--version"]);
  if (!hasSha256Sum && !hasShasum) {
    t.skip("no standard SHA-256 checking utility is installed");
    return;
  }
  const result = hasSha256Sum
    ? spawnSync("sha256sum", ["-c", "SHA256SUMS.txt"], {
        cwd: directory,
        encoding: "utf8",
      })
    : spawnSync("shasum", ["-a", "256", "-c", "SHA256SUMS.txt"], {
        cwd: directory,
        encoding: "utf8",
      });
  expectOk(result);
  assert.match(result.stdout, /factory-linux-x86_64\.tar\.gz: OK/);
  assert.match(result.stdout, /factory-windows-x86_64\.zip: OK/);
});

test("manifest ordering is deterministic regardless of input order", (t) => {
  const directory = workspace(t);
  const reversed = [...ARCHIVES].reverse();
  generateAll(directory, reversed);
  manifestFor(directory, reversed);
  const names = linesOf(join(directory, "SHA256SUMS.txt")).map(
    (line) => line.split("  ")[1],
  );
  assert.deepEqual(names, [...ARCHIVES].sort());
});

test("manifest entries contain basenames only", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  for (const line of linesOf(manifest)) {
    assert.match(line, /^[0-9a-f]{64}  [^/\\]+$/);
    const name = line.split("  ")[1];
    assert.ok(ARCHIVES.includes(name));
  }
});

test("shasum generation matches the helper digest", (t) => {
  if (!commandAvailable("shasum", ["--version"])) {
    t.skip("shasum is not installed");
    return;
  }
  const directory = workspace(t);
  const file = join(directory, "fixture.bin");
  writeFileSync(file, "shasum fixture\n");
  const generated = spawnSync("shasum", ["-a", "256", file], {
    encoding: "utf8",
  });
  expectOk(generated);
  const externalDigest = generated.stdout.split(/\s+/)[0];
  expectOk(run(["generate", file], directory));
  assert.equal(
    readFileSync(`${file}.sha256`, "utf8").split("  ")[0],
    externalDigest,
  );
});

test("rejects an archive modified after checksum generation", (t) => {
  const directory = workspace(t);
  generateAll(directory, ["factory-linux-x86_64.tar.gz"]);
  appendFileSync(join(directory, "factory-linux-x86_64.tar.gz"), "tampered");
  expectFailure(
    run(["verify", "factory-linux-x86_64.tar.gz"], directory),
    /checksum mismatch/,
  );
});

test("rejects a checksum modified after generation", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(directory, name, ZERO_DIGEST);
  expectFailure(run(["verify", name], directory), /checksum mismatch/);
});

test("rejects a missing archive", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  unlinkSync(join(directory, name));
  expectFailure(run(["verify", name], directory), /archive is missing/);
});

test("rejects a missing .sha256 file", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  unlinkSync(join(directory, `${name}.sha256`));
  expectFailure(run(["verify", name], directory), /checksum file is missing/);
});

test("rejects an invalid hexadecimal digest", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(directory, name, "g".repeat(64));
  expectFailure(run(["verify", name], directory), /invalid SHA-256 entry/);
});

test("rejects a truncated digest", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(directory, name, "a".repeat(63));
  expectFailure(run(["verify", name], directory), /invalid SHA-256 entry/);
});

test("rejects an empty checksum file", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeFileSync(join(directory, `${name}.sha256`), "");
  expectFailure(run(["verify", name], directory), /checksum file is empty/);
});

test("rejects a checksum file with multiple entries", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  const checksum = join(directory, `${name}.sha256`);
  const line = readFileSync(checksum, "utf8").trimEnd();
  writeFileSync(checksum, `${line}\n${line}\n`);
  expectFailure(
    run(["verify", name], directory),
    /checksum file must contain exactly one entry/,
  );
});

test("rejects a checksum file without a trailing newline", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  const checksum = join(directory, `${name}.sha256`);
  writeFileSync(checksum, readFileSync(checksum, "utf8").trimEnd());
  expectFailure(run(["verify", name], directory), /must end with a newline/);
});

test("rejects duplicate manifest entries", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = linesOf(manifest);
  lines[1] = lines[0];
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /duplicate manifest entry/,
  );
});

test("rejects a checksum referring to another archive filename", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(directory, name, digestFrom(directory, name), "other.tar.gz");
  expectFailure(
    run(["verify", name], directory),
    /refers to other\.tar\.gz/,
  );
});

test("rejects an archive path containing release/", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(
    directory,
    name,
    digestFrom(directory, name),
    `release/${name}`,
  );
  expectFailure(run(["verify", name], directory), /invalid SHA-256 entry/);
});

test("rejects an absolute path in a checksum", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  writeChecksum(
    directory,
    name,
    digestFrom(directory, name),
    `/tmp/${name}`,
  );
  expectFailure(run(["verify", name], directory), /invalid SHA-256 entry/);
});

test("rejects a checksum-of-checksum manifest entry", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = linesOf(manifest);
  lines[0] = `${ZERO_DIGEST}  ${ARCHIVES[0]}.sha256`;
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /manifest entry mismatch/,
  );
});

test("rejects a manifest entry referring to a nonexistent archive", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = linesOf(manifest);
  lines[0] = `${ZERO_DIGEST}  ghost.tar.gz`;
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /manifest entry mismatch/,
  );
});

test("rejects a manifest with fewer than four expected entries", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = linesOf(manifest).slice(0, 3);
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /exactly 4 entries, found 3/,
  );
});

test("rejects a manifest with an unexpected additional archive", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = [
    ...linesOf(manifest),
    `${ZERO_DIGEST}  factory-extra.tar.gz`,
  ];
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /exactly 4 entries, found 5/,
  );
});

test("rejects a consolidated manifest containing a wrong digest", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  const lines = linesOf(manifest);
  const name = lines[0].split("  ")[1];
  lines[0] = `${ZERO_DIGEST}  ${name}`;
  writeFileSync(manifest, `${lines.join("\n")}\n`);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /manifest entry mismatch/,
  );
});

test("rejects inconsistent per-archive and consolidated manifests", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const manifest = manifestFor(directory);
  writeChecksum(directory, ARCHIVES[0], ZERO_DIGEST);
  expectFailure(
    run(["verify-manifest", manifest, ...ARCHIVES], directory),
    /checksum mismatch/,
  );
});

test("detects archive and checksum tampering end to end", (t) => {
  const directory = workspace(t);
  const name = "factory-linux-x86_64.tar.gz";
  generateAll(directory, [name]);
  expectOk(run(["verify", name], directory));
  const archive = join(directory, name);
  const original = readFileSync(archive);
  const tampered = Buffer.from(original);
  tampered[0] ^= 0xff;
  writeFileSync(archive, tampered);
  expectFailure(run(["verify", name], directory), /checksum mismatch/);
  writeFileSync(archive, original);
  expectOk(run(["verify", name], directory));
  writeChecksum(directory, name, ZERO_DIGEST);
  expectFailure(run(["verify", name], directory), /checksum mismatch/);
});

test("finalize-release verifies all artifacts and writes the manifest", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  expectOk(run(["finalize-release", directory], directory));
  const manifest = join(directory, "SHA256SUMS.txt");
  assert.ok(existsSync(manifest));
  expectOk(run(["verify-manifest", manifest, ...ARCHIVES], directory));
});

test("finalize-release rejects an unexpected release file", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  writeFileSync(join(directory, "notes.txt"), "unexpected\n");
  expectFailure(
    run(["finalize-release", directory], directory),
    /unexpected entry in release directory: notes\.txt/,
  );
});

test("finalize-release rejects a missing archive", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  unlinkSync(join(directory, ARCHIVES[0]));
  expectFailure(
    run(["finalize-release", directory], directory),
    /missing release archive/,
  );
});

test("finalize-release rejects a missing per-archive checksum", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  unlinkSync(join(directory, `${ARCHIVES[0]}.sha256`));
  expectFailure(
    run(["finalize-release", directory], directory),
    /missing release checksum/,
  );
});

test("finalize-release rejects an inventory containing paths", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const inventory = join(directory, "inventory.txt");
  writeFileSync(
    inventory,
    `release/${ARCHIVES[0]}\n${ARCHIVES[1]}\n${ARCHIVES[2]}\n${ARCHIVES[3]}\n`,
  );
  expectFailure(
    run(["finalize-release", directory, inventory], directory),
    /basenames only/,
  );
});

test("finalize-release rejects duplicate inventory entries", (t) => {
  const directory = workspace(t);
  generateAll(directory);
  const inventory = join(directory, "inventory.txt");
  writeFileSync(
    inventory,
    `${ARCHIVES[0]}\n${ARCHIVES[0]}\n${ARCHIVES[1]}\n${ARCHIVES[2]}\n`,
  );
  expectFailure(
    run(["finalize-release", directory, inventory], directory),
    /duplicate artifact inventory entry/,
  );
});

test("release workflow verifies checksums before publication", () => {
  const workflow = readText(WORKFLOW_PATH);
  assert.match(workflow, /node scripts\/release-checksums\.mjs generate/);
  assert.match(workflow, /node scripts\/release-checksums\.mjs verify/);
  assert.match(
    workflow,
    /node scripts\/release-checksums\.mjs verify-release release/,
  );
  assert.match(
    workflow,
    /node scripts\/release-checksums\.mjs finalize-release release/,
  );
  assert.match(workflow, /sha256sum -c SHA256SUMS\.txt/);
  assert.doesNotMatch(workflow, /cat release\/\*\.sha256/);
  assert.doesNotMatch(workflow, /release\/\*/);
  const generateIndex = workflow.indexOf(
    "node scripts/release-checksums.mjs generate",
  );
  const verifyIndex = workflow.indexOf(
    "node scripts/release-checksums.mjs verify",
  );
  const uploadIndex = workflow.indexOf("uses: actions/upload-artifact@v4");
  const inventoryIndex = workflow.indexOf(
    "node scripts/release-checksums.mjs verify-release release",
  );
  const contentIndex = workflow.indexOf("Verify all archive contents");
  const finalizeIndex = workflow.indexOf(
    "node scripts/release-checksums.mjs finalize-release release",
  );
  const checkIndex = workflow.indexOf("sha256sum -c SHA256SUMS.txt");
  const publishIndex = workflow.indexOf("gh release create");
  assert.ok(generateIndex >= 0);
  assert.ok(generateIndex < verifyIndex);
  assert.ok(verifyIndex < uploadIndex);
  assert.ok(inventoryIndex < contentIndex);
  assert.ok(contentIndex < finalizeIndex);
  assert.ok(finalizeIndex < checkIndex);
  assert.ok(checkIndex < publishIndex);
});

test("release artifact inventory is explicit and complete", () => {
  const inventory = readText(INVENTORY_PATH).trimEnd().split("\n");
  assert.deepEqual(inventory, ARCHIVES);
});

test("release build matrix matches the explicit artifact inventory", () => {
  const workflow = readText(WORKFLOW_PATH);
  const archives = [...workflow.matchAll(/^\s+archive: (\S+)$/gm)].map(
    (match) => match[1],
  );
  assert.deepEqual(archives.sort(), [...ARCHIVES].sort());
});
