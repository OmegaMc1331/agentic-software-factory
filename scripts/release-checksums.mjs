#!/usr/bin/env node
import { createHash } from "node:crypto";
import {
  createReadStream,
  existsSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const DIGEST_PATTERN = /^[0-9a-f]{64}$/;
const ENTRY_PATTERN = /^([0-9a-f]{64})  ([^/\\]+)$/;
const DEFAULT_INVENTORY = fileURLToPath(
  new URL("release-artifacts.txt", import.meta.url),
);

function fail(message) {
  throw new Error(message);
}

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function readCanonicalLines(file, label, { allowCrlf = false } = {}) {
  if (!isFile(file)) {
    fail(`${label} is missing: ${file}`);
  }
  const data = readFileSync(file);
  if (data.length === 0) {
    fail(`${label} is empty: ${file}`);
  }
  let text = data.toString("utf8");
  if (allowCrlf) {
    text = text.replace(/\r\n/g, "\n");
  }
  if (!text.endsWith("\n")) {
    fail(`${label} must end with a newline: ${file}`);
  }
  if (text.includes("\r")) {
    fail(`${label} contains carriage returns: ${file}`);
  }
  const lines = text.slice(0, -1).split("\n");
  if (lines.some((line) => line.length === 0)) {
    fail(`${label} contains an empty entry: ${file}`);
  }
  return lines;
}

function parseEntry(line, label) {
  const match = ENTRY_PATTERN.exec(line);
  if (!match) {
    fail(`${label} contains an invalid SHA-256 entry: ${line}`);
  }
  const digest = match[1];
  const name = match[2];
  if (!DIGEST_PATTERN.test(digest)) {
    fail(`${label} contains an invalid SHA-256 digest: ${digest}`);
  }
  return { digest, name };
}

function formatEntries(entries) {
  return `${entries.map(({ digest, name }) => `${digest}  ${name}`).join("\n")}\n`;
}

function compareEntries(left, right) {
  if (left.name < right.name) return -1;
  if (left.name > right.name) return 1;
  return 0;
}

async function sha256File(file) {
  return await new Promise((resolveDigest, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(file);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolveDigest(hash.digest("hex")));
  });
}

async function verifyArchive(archivePath) {
  const archive = resolve(archivePath);
  if (!isFile(archive)) {
    fail(`archive is missing: ${archivePath}`);
  }
  const name = basename(archive);
  const checksumPath = `${archive}.sha256`;
  const lines = readCanonicalLines(checksumPath, "checksum file");
  if (lines.length !== 1) {
    fail(`checksum file must contain exactly one entry: ${checksumPath}`);
  }
  const entry = parseEntry(lines[0], "checksum file");
  if (entry.name !== name) {
    fail(
      `checksum file ${checksumPath} refers to ${entry.name}, expected ${name}`,
    );
  }
  const actual = await sha256File(archive);
  if (actual !== entry.digest) {
    fail(
      `checksum mismatch for ${name}: expected ${entry.digest}, got ${actual}`,
    );
  }
  return { name, digest: actual };
}

async function generate(archivePath) {
  const archive = resolve(archivePath);
  if (!isFile(archive)) {
    fail(`archive is missing: ${archivePath}`);
  }
  const name = basename(archive);
  const digest = await sha256File(archive);
  const checksumPath = `${archive}.sha256`;
  const temporary = `${checksumPath}.tmp-${process.pid}`;
  writeFileSync(temporary, `${digest}  ${name}\n`, "utf8");
  renameSync(temporary, checksumPath);
  const verified = await verifyArchive(archive);
  console.log(`OK: wrote and verified ${checksumPath}`);
  return verified;
}

async function manifest(outputPath, archivePaths) {
  if (archivePaths.length === 0) {
    fail("at least one archive is required");
  }
  const names = new Set();
  const entries = [];
  for (const archivePath of archivePaths) {
    const entry = await verifyArchive(archivePath);
    if (names.has(entry.name)) {
      fail(`duplicate archive in manifest: ${entry.name}`);
    }
    names.add(entry.name);
    entries.push(entry);
  }
  entries.sort(compareEntries);
  const output = resolve(outputPath);
  const temporary = `${output}.tmp-${process.pid}`;
  writeFileSync(temporary, formatEntries(entries), "utf8");
  try {
    await verifyManifest(temporary, archivePaths);
    renameSync(temporary, output);
  } finally {
    if (existsSync(temporary)) {
      unlinkSync(temporary);
    }
  }
  console.log(`OK: wrote ${output} with ${entries.length} entries`);
  return entries;
}

async function verifyManifest(manifestPath, archivePaths) {
  if (archivePaths.length === 0) {
    fail("at least one archive is required");
  }
  const manifestFile = resolve(manifestPath);
  const lines = readCanonicalLines(manifestFile, "manifest");
  if (lines.length !== archivePaths.length) {
    fail(
      `manifest must contain exactly ${archivePaths.length} entries, found ${lines.length}`,
    );
  }
  const actualEntries = lines.map((line, index) =>
    parseEntry(line, `manifest line ${index + 1}`),
  );
  const names = new Set();
  for (const entry of actualEntries) {
    if (names.has(entry.name)) {
      fail(`duplicate manifest entry: ${entry.name}`);
    }
    names.add(entry.name);
  }
  const expectedEntries = [];
  for (const archivePath of archivePaths) {
    expectedEntries.push(await verifyArchive(archivePath));
  }
  expectedEntries.sort(compareEntries);
  const sortedActual = [...actualEntries].sort(compareEntries);
  for (let index = 0; index < expectedEntries.length; index += 1) {
    const expected = expectedEntries[index];
    const actual = sortedActual[index];
    if (actual.name !== expected.name || actual.digest !== expected.digest) {
      fail(
        `manifest entry mismatch for ${expected.name}: expected ${expected.digest}, found ${actual.digest}`,
      );
    }
  }
  const expectedText = formatEntries(expectedEntries);
  const actualText = readFileSync(manifestFile, "utf8");
  if (actualText !== expectedText) {
    fail(`manifest is not in deterministic filename order: ${manifestFile}`);
  }
  return expectedEntries;
}

function readInventory(inventoryPath) {
  const lines = readCanonicalLines(inventoryPath, "artifact inventory", {
    allowCrlf: true,
  });
  const names = new Set();
  for (const line of lines) {
    if (
      line !== line.trim() ||
      line === "." ||
      line === ".." ||
      line.includes("/") ||
      line.includes("\\") ||
      line.includes(":")
    ) {
      fail(`artifact inventory must contain basenames only: ${line}`);
    }
    if (names.has(line)) {
      fail(`duplicate artifact inventory entry: ${line}`);
    }
    names.add(line);
  }
  return lines;
}

async function verifyRelease(directory, inventoryPath = DEFAULT_INVENTORY) {
  const root = resolve(directory);
  if (!existsSync(root) || !statSync(root).isDirectory()) {
    fail(`release directory is missing: ${directory}`);
  }
  const names = readInventory(inventoryPath);
  const allowed = new Set(["SHA256SUMS.txt"]);
  for (const name of names) {
    allowed.add(name);
    allowed.add(`${name}.sha256`);
  }
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (!entry.isFile() || !allowed.has(entry.name)) {
      fail(`unexpected entry in release directory: ${entry.name}`);
    }
  }
  const archives = names.map((name) => join(root, name));
  for (const archive of archives) {
    if (!isFile(archive)) {
      fail(`missing release archive: ${basename(archive)}`);
    }
    if (!isFile(`${archive}.sha256`)) {
      fail(`missing release checksum: ${basename(archive)}.sha256`);
    }
    await verifyArchive(archive);
  }
  console.log(
    `OK: verified ${archives.length} release archives and their checksums`,
  );
  return archives;
}

async function finalizeRelease(directory, inventoryPath = DEFAULT_INVENTORY) {
  const root = resolve(directory);
  const archives = await verifyRelease(root, inventoryPath);
  const manifestPath = join(root, "SHA256SUMS.txt");
  await manifest(manifestPath, archives);
  console.log(
    `OK: release artifacts verified and ${basename(manifestPath)} generated`,
  );
  return manifestPath;
}

function usage() {
  console.error(
    [
      "Usage:",
      "  node scripts/release-checksums.mjs generate <archive>",
      "  node scripts/release-checksums.mjs verify <archive>",
      "  node scripts/release-checksums.mjs manifest <output> <archive...>",
      "  node scripts/release-checksums.mjs verify-manifest <manifest> <archive...>",
      "  node scripts/release-checksums.mjs verify-release <directory> [inventory]",
      "  node scripts/release-checksums.mjs finalize-release <directory> [inventory]",
    ].join("\n"),
  );
}

async function main() {
  const [command, ...args] = process.argv.slice(2);
  switch (command) {
    case "generate":
      if (args.length !== 1) fail("generate requires one archive");
      await generate(args[0]);
      break;
    case "verify":
      if (args.length !== 1) fail("verify requires one archive");
      console.log(
        `OK: verified ${args[0]} ${(await verifyArchive(args[0])).digest}`,
      );
      break;
    case "manifest":
      if (args.length < 2) {
        fail("manifest requires an output file and at least one archive");
      }
      await manifest(args[0], args.slice(1));
      break;
    case "verify-manifest": {
      if (args.length < 2) {
        fail("verify-manifest requires a manifest and at least one archive");
      }
      const entries = await verifyManifest(args[0], args.slice(1));
      console.log(
        `OK: verified ${args[0]} against ${entries.length} archives`,
      );
      break;
    }
    case "verify-release":
      if (args.length < 1 || args.length > 2) {
        fail("verify-release requires a directory and optional inventory");
      }
      await verifyRelease(args[0], args[1]);
      break;
    case "finalize-release":
      if (args.length < 1 || args.length > 2) {
        fail("finalize-release requires a directory and optional inventory");
      }
      await finalizeRelease(args[0], args[1]);
      break;
    default:
      usage();
      fail(`unknown command: ${command ?? ""}`);
  }
}

const entryPoint = process.argv[1]
  ? pathToFileURL(resolve(process.argv[1])).href
  : "";
if (entryPoint === import.meta.url) {
  main().catch((error) => {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  });
}

export {
  finalizeRelease,
  generate,
  manifest,
  parseEntry,
  readCanonicalLines,
  sha256File,
  verifyArchive,
  verifyManifest,
  verifyRelease,
};
