import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import struct
import subprocess
import tarfile
import time
import tomllib
import zipfile

from dependencies import inventory
from run import tracked_fingerprint

ROOT = Path(__file__).resolve().parents[2]
TARGETS = {
    "x86_64-pc-windows-msvc": ("Windows", "AMD64", "factory-windows-x86_64.zip", "factory.exe"),
    "x86_64-unknown-linux-gnu": ("Linux", "x86_64", "factory-linux-x86_64.tar.gz", "factory"),
    "aarch64-apple-darwin": ("Darwin", "arm64", "factory-macos-aarch64.tar.gz", "factory"),
    "x86_64-apple-darwin": ("Darwin", "x86_64", "factory-macos-x86_64.tar.gz", "factory"),
}
DOCUMENTS = ("INSTALLATION.md", "UPGRADE.md", "SECURITY.md", "KNOWN-ISSUES.md", "USER-GUIDE.md")
DEPENDENCY_FILES = ("THIRD-PARTY-NOTICES.txt", "sbom.cdx.json", "license-review.json")
LOCKFILES = ("Cargo.lock", "apps/dashboard/package-lock.json", "rust-toolchain.toml", ".node-version")


def launcher_name(target):
    return "start-workspace.ps1" if TARGETS[target][0] == "Windows" else "start-workspace.sh"


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def sha(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def host(target):
    expected_os, expected_arch, _, _ = TARGETS[target]
    actual_arch = platform.machine()
    if (platform.system(), actual_arch) != (expected_os, expected_arch):
        raise ValueError(f"Native host mismatch: {platform.system()} {actual_arch}; expected {expected_os} {expected_arch}")
    rust = subprocess.check_output(["rustc", "-vV"], text=True)
    if f"host: {target}" not in rust or not rust.startswith("rustc 1.99.1 "):
        raise ValueError("Pinned Rust version/native host mismatch")
    print(rust.strip())


def verify_machine(data, target):
    if target.endswith("windows-msvc"):
        if data[:2] != b"MZ" or len(data) < 64:
            raise ValueError("Expected PE executable")
        offset = struct.unpack_from("<I", data, 60)[0]
        if offset < 64 or offset > len(data) - 26 or data[offset:offset + 4] != b"PE\0\0" or struct.unpack_from("<H", data, offset + 4)[0] != 0x8664:
            raise ValueError("Expected x86_64 PE executable")
    elif target.endswith("linux-gnu"):
        if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != 62:
            raise ValueError("Expected little-endian x86_64 ELF executable")
    else:
        if len(data) < 32 or data[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError("Expected native 64-bit Mach-O executable")
        expected = 0x0100000C if target.startswith("aarch64") else 0x01000007
        if struct.unpack_from("<I", data, 4)[0] != expected:
            raise ValueError("Wrong Mach-O architecture")


def valid_member(name):
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and ".." not in path.parts and "\\" not in name and ":" not in name and name == path.as_posix()


def unpack(archive, destination):
    if destination.exists():
        raise ValueError("Extraction destination must be new")
    entries = []
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as package:
            if len(package.infolist()) > 64 or sum(item.file_size for item in package.infolist()) > 200_000_000:
                raise ValueError("Package inventory exceeds limits")
            if len({item.filename for item in package.infolist()}) != len(package.infolist()):
                raise ValueError("Repeated package path")
            for item in package.infolist():
                if not valid_member(item.filename) or item.is_dir() or item.file_size > 150_000_000 or (item.external_attr >> 16) & 0o170000 == 0o120000:
                    raise ValueError("Unsafe package entry")
                entries.append((item.filename, package.read(item), (item.external_attr >> 16) & 0o777))
    else:
        with tarfile.open(archive) as package:
            total = 0
            for number, item in enumerate(package):
                total += item.size
                if number >= 64 or total > 200_000_000:
                    raise ValueError("Package inventory exceeds limits")
                if not valid_member(item.name) or not item.isfile() or item.size > 150_000_000:
                    raise ValueError("Unsafe package entry")
                entries.append((item.name, package.extractfile(item).read(), item.mode & 0o777))
    if len(entries) > 64 or len({name for name, _, _ in entries}) != len(entries) or sum(len(data) for _, data, _ in entries) > 200_000_000:
        raise ValueError("Package inventory exceeds limits or repeats paths")
    destination.mkdir(parents=True)
    for name, data, mode in entries:
        file = destination / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(data)
        file.chmod(mode or 0o644)
    manifest = json.loads((destination / "PACKAGE-MANIFEST.json").read_text(encoding="utf-8"))
    expected = {item["path"] for item in manifest["files"]} | {"PACKAGE-MANIFEST.json"}
    if expected != {name for name, _, _ in entries}:
        raise ValueError("Package inventory differs from manifest")
    for item in manifest["files"]:
        if sha(destination / item["path"]) != item["sha256"]:
            raise ValueError("Packaged file checksum mismatch")
    return manifest


def build(args):
    host(args.target)
    source_paths = ["crates", "apps/dashboard", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".node-version", "scripts", ".github/workflows", "install.ps1", "install.sh", "docs/v2/release/INSTALLATION.md", "docs/v2/release/UPGRADE.md", "docs/v2/release/SECURITY.md", "docs/v2/release/KNOWN-ISSUES.md", "docs/v2/release/USER-GUIDE.md"]
    if git("status", "--porcelain", "--", *source_paths):
        raise ValueError("Freeze and commit candidate build inputs before packaging")
    args.output.mkdir(parents=True, exist_ok=True)
    _, _, name, executable = TARGETS[args.target]
    archive = args.output / name
    if archive.exists():
        raise ValueError("Preserve prior candidate packages; select a new output directory")
    binary = args.binary or Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "release" / executable
    build_record = json.loads(args.build_record.read_text(encoding="utf-8"))
    if build_record.get("result") != "PASS" or not build_record.get("sourceUnchanged") or build_record["sourceCommit"] != git("rev-parse", "HEAD") or build_record["finalSourceFingerprint"] != tracked_fingerprint() or sha(binary) != build_record["binary"]["sha256"]:
        raise ValueError("Require a passing unchanged-source build record matching this exact binary and source")
    verify_machine(binary.read_bytes(), args.target)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if subprocess.check_output([str(binary.resolve()), "--version"], text=True).strip() != f"factory {version}":
        raise ValueError("Binary version does not match source")
    inventory(args.output / "dependencies")
    files = {executable: (binary.read_bytes(), 0o755), "LICENSE": ((ROOT / "LICENSE").read_bytes(), 0o644)}
    for document in DOCUMENTS:
        files[document] = ((ROOT / "docs/v2/release" / document).read_bytes(), 0o644)
    for document in DEPENDENCY_FILES:
        files[document] = ((args.output / "dependencies" / document).read_bytes(), 0o644)
    launcher = ROOT / "scripts/v2_p14" / launcher_name(args.target)
    files[launcher.name] = (launcher.read_bytes(), 0o755)
    epoch = int(git("show", "-s", "--format=%ct", "HEAD"))
    manifest = {"schemaVersion": 1, "version": version, "sourceCommit": git("rev-parse", "HEAD"), "sourceTree": git("rev-parse", "HEAD^{tree}"), "sourceFingerprint": build_record["sourceFingerprint"], "target": args.target, "buildProfile": "release with LTO", "sourceDateEpoch": epoch, "signing": "UNSIGNED", "notarization": "NOT QUALIFIED", "embeddedAssets": build_record["embeddedAssets"], "lockfiles": {file: sha(ROOT / file) for file in LOCKFILES}, "files": [{"path": name, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data), "mode": mode} for name, (data, mode) in sorted(files.items())]}
    files["PACKAGE-MANIFEST.json"] = ((json.dumps(manifest, indent=2) + "\n").encode(), 0o644)
    write_archive(archive, files, epoch)
    archive_sha256 = sha(archive)
    archive.with_name(archive.name + ".sha256").write_bytes(f"{archive_sha256}  {archive.name}\n".encode("ascii"))
    record = {"schemaVersion": 1, "artifact": archive.name, "sha256": archive_sha256, "bytes": archive.stat().st_size, **manifest}
    (args.output / "artifact.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: record[key] for key in ["artifact", "target", "sha256", "bytes", "sourceCommit"]}))


def write_archive(archive, files, epoch):
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as package:
            for name, (data, mode) in sorted(files.items()):
                info = zipfile.ZipInfo(name, time.gmtime(max(315532800, epoch))[:6])
                info.create_system = 3
                info.external_attr = (0o100000 | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                package.writestr(info, data)
    else:
        with archive.open("wb") as stream, gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=epoch) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as package:
            for name, (data, mode) in sorted(files.items()):
                info = tarfile.TarInfo(name)
                info.size, info.mode, info.mtime = len(data), mode, epoch
                package.addfile(info, io.BytesIO(data))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["host", "build"])
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--build-record", type=Path, default=ROOT / ".factory/v2/p14/native/build-results.json")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.operation == "host":
        host(args.target)
    else:
        if args.output is None:
            parser.error("--output is required for build")
        build(args)
