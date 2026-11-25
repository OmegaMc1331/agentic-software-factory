"""Validate the publisher's actual bytes and seal an exact, flat upload set."""

from dataclasses import dataclass
import gzip
import io
import os
from pathlib import Path
import re
import stat
import struct
import tarfile
import tempfile
import tomllib
import zipfile

import package
from ci_privacy import (
    JSON_LIMIT, MEMBER_LIMIT, OID, SHA256, TOTAL_LIMIT, PrivacyError,
    bounded_read, digest, encode, is_link, parse, screen,
)


RECEIPT = "privacy-receipt.json"
CHECKSUMS = "SHA256SUMS.txt"
FONT = re.compile(r"assets/((?:inter|jetbrains-mono)-latin-wght-normal)-[a-zA-Z0-9_-]+\.woff2")
WEB_ASSET = re.compile(r"assets/[a-zA-Z0-9_.-]+\.(?:js|css)")
WEB_FILES = {"index.html", "projects.html", "workspace.html", "licenses/inter-OFL.txt", "licenses/jetbrains-mono-OFL.txt"}
NESTED_MAGIC = (b"PK\x03\x04", b"PK\x05\x06", b"\x1f\x8b", b"7z\xbc\xaf", b"Rar!", b"\xfd7zXZ", b"BZh", b"\x28\xb5\x2f\xfd")


@dataclass(frozen=True)
class Source:
    commit: str
    tree: str
    epoch: int
    version: str


def source_identity(root):
    version = tomllib.loads(bounded_read(root / "Cargo.toml", JSON_LIMIT).decode())["workspace"]["package"]["version"]
    commit = package.git("rev-parse", "HEAD")
    tree = package.git("rev-parse", "HEAD^{tree}")
    epoch = int(package.git("show", "-s", "--format=%ct", "HEAD"))
    if not OID.fullmatch(commit) or not OID.fullmatch(tree) or os.environ.get("GITHUB_SHA", commit) != commit:
        raise PrivacyError("Checkout identity differs from qualification source")
    return Source(commit, tree, epoch, version)


def directory(path):
    info = path.lstat()
    if is_link(info) or not stat.S_ISDIR(info.st_mode):
        raise PrivacyError("Unexpected publication directory type")


def flat_files(path, names):
    directory(path)
    if {entry.name for entry in path.iterdir()} != names:
        raise PrivacyError("Unexpected publication inventory")
    return {name: bounded_read(path / name, MEMBER_LIMIT if name.endswith((".zip", ".tar.gz")) else JSON_LIMIT)
            for name in sorted(names)}


def zip_members(data, names):
    if len(data) < 22:
        raise PrivacyError("Incomplete ZIP header")
    signature, disk, start, entries, total, central_size, offset, comment = struct.unpack("<4s4H2IH", data[-22:])
    if (signature != b"PK\x05\x06" or disk or start or comment or entries != len(names)
            or total != entries or central_size > 65536 or offset + central_size + 22 != len(data)):
        raise PrivacyError("Unsupported ZIP directory")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        inventory = archive.infolist()
        if archive.comment or len(inventory) != len(names) or {item.filename for item in inventory} != names:
            raise PrivacyError("Unexpected ZIP inventory")
        if sum(item.file_size for item in inventory) > TOTAL_LIMIT:
            raise PrivacyError("ZIP exceeds expanded size limit")
        entries = {}
        for item in inventory:
            mode = item.external_attr >> 16
            if (stat.S_IFMT(mode) != stat.S_IFREG or item.file_size > MEMBER_LIMIT
                    or item.extra or item.comment or item.flag_bits & 1
                    or not package.valid_member(item.filename)):
                raise PrivacyError("Unsupported ZIP entry")
            entries[item.filename] = (archive.read(item), mode & 0o777)
        return entries


def tar_members(data, names):
    # Bound the entire decompressed stream before tarfile interprets any
    # headers, including PAX records and tar padding.
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as stream:
        expanded = stream.read(TOTAL_LIMIT + 1)
    if len(expanded) > TOTAL_LIMIT:
        raise PrivacyError("Tar exceeds expanded size limit")
    entries = {}
    with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as archive:
        for item in archive:
            if (item.name not in names or item.name in entries or item.type != tarfile.REGTYPE
                    or item.size > MEMBER_LIMIT or not package.valid_member(item.name)):
                raise PrivacyError("Unsupported tar entry")
            with archive.extractfile(item) as stream:
                entries[item.name] = (stream.read(), item.mode)
    if set(entries) != names:
        raise PrivacyError("Unexpected tar inventory")
    return entries


def inspect_assets(root, expected):
    dist = root / "apps/dashboard/dist"
    directory(dist)
    assets = []
    for folder, directories, files in dist.walk(follow_symlinks=False):
        for name in directories:
            directory(folder / name)
        for name in files:
            file = folder / name
            relative = file.relative_to(dist).as_posix()
            data = bounded_read(file)
            font = FONT.fullmatch(relative)
            if font:
                family = font[1].removesuffix("-latin-wght-normal")
                origin = root / "apps/dashboard/node_modules/@fontsource-variable" / family / "files" / (font[1] + ".woff2")
                if data[:4] != b"wOF2" or data != bounded_read(origin):
                    raise PrivacyError("Unsupported embedded font")
            elif relative in WEB_FILES or WEB_ASSET.fullmatch(relative):
                screen(data)
            else:
                raise PrivacyError("Unexpected embedded asset type")
            assets.append({"path": relative, "bytes": len(data), "sha256": digest(data)})
    if not {"index.html", "projects.html", "workspace.html"} <= {item["path"] for item in assets}:
        raise PrivacyError("Missing embedded entry points")
    if sorted(assets, key=lambda item: item["path"]) != sorted(expected, key=lambda item: item["path"]):
        raise PrivacyError("Embedded asset inventory mismatch")


def inspect_distribution(data, target, source, root, scratch):
    _, _, name, executable = package.TARGETS[target]
    launcher = package.launcher_name(target)
    original = {filename: root / "docs/v2/release" / filename for filename in package.DOCUMENTS}
    original.update({"LICENSE": root / "LICENSE", launcher: root / "scripts/v2_p14" / launcher})
    manifest_name = "PACKAGE-MANIFEST.json"
    names = set(original) | set(package.DEPENDENCY_FILES) | {executable, manifest_name}
    entries = zip_members(data, names) if name.endswith(".zip") else tar_members(data, names)
    for filename, (content, mode) in entries.items():
        if content.startswith(NESTED_MAGIC) or content[257:262] == b"ustar" or not content:
            raise PrivacyError("Unsupported package payload")
        if mode != (0o755 if filename in {executable, launcher} else 0o644):
            raise PrivacyError("Unexpected package mode")
        screen(content, binary=filename == executable)
    binary = entries[executable][0]
    package.verify_machine(binary, target)
    for filename, path in original.items():
        if entries[filename][0] != bounded_read(path, JSON_LIMIT):
            raise PrivacyError("Packaged license or document differs from public source")

    manifest = parse(entries[manifest_name][0])
    expected = {
        "schemaVersion": 1, "version": source.version, "sourceCommit": source.commit, "sourceTree": source.tree,
        "sourceDateEpoch": source.epoch, "target": target, "buildProfile": "release with LTO",
        "signing": "UNSIGNED", "notarization": "NOT QUALIFIED",
        "lockfiles": {file: digest(bounded_read(root / file, JSON_LIMIT)) for file in package.LOCKFILES},
        "files": [{"path": file, "sha256": digest(content), "bytes": len(content), "mode": mode}
                  for file, (content, mode) in sorted(entries.items()) if file != manifest_name],
    }
    fingerprint = manifest.get("sourceFingerprint")
    if (type(fingerprint) is not str or not SHA256.fullmatch(fingerprint)
            or set(manifest) != set(expected) | {"sourceFingerprint", "embeddedAssets"}
            or any(encode(manifest[key]) != encode(value) for key, value in expected.items())):
        raise PrivacyError("Package manifest identity mismatch")
    inspect_assets(root, manifest["embeddedAssets"])
    bom = parse(entries["sbom.cdx.json"][0])
    review = parse(entries["license-review.json"][0])
    if (bom.get("bomFormat") != "CycloneDX" or bom.get("specVersion") != "1.6"
            or type(bom.get("components")) is not list or not bom["components"]
            or review.get("componentCount") != len(bom["components"])):
        raise PrivacyError("SBOM or license inventory invalid")

    # Reproduce the producer's encoding on the same host. This also checks
    # headers, extended metadata, padding and trailing bytes.
    with tempfile.TemporaryDirectory(dir=scratch, prefix="archive-check-") as tmp:
        canonical = Path(tmp) / name
        package.write_archive(canonical, entries, source.epoch)
        if bounded_read(canonical) != data:
            raise PrivacyError("Noncanonical package encoding")
    return digest(binary), len(manifest["embeddedAssets"])


def names_for(scope):
    if scope in package.TARGETS:
        name = package.TARGETS[scope][2]
        return {name, name + ".sha256"}
    if scope != "inventory":
        raise PrivacyError("Unknown artifact scope")
    return {CHECKSUMS} | {file for target in package.TARGETS for file in names_for(target)}


def receipt_for(files, source, scope):
    if set(files) != names_for(scope) or not OID.fullmatch(source):
        raise PrivacyError("Unexpected publication receipt inputs")
    return encode({"schemaVersion": 1, "result": "PASS", "source": source, "scope": scope,
                   "files": {name: {"sha256": digest(data), "bytes": len(data)} for name, data in sorted(files.items())}})


def stage(files, publish, seal, source, scope):
    receipt = receipt_for(files, source, scope)
    publish.mkdir(parents=True)
    for filename, content in files.items():
        (publish / filename).write_bytes(content)
    (publish / RECEIPT).write_bytes(receipt)
    with seal.open("xb") as internal:
        internal.write(receipt)


def read_screened(publish, source, scope, seal=None):
    files = flat_files(publish, names_for(scope) | {RECEIPT})
    receipt = files.pop(RECEIPT)
    if (receipt != receipt_for(files, source, scope)
            or (seal is not None and receipt != bounded_read(seal, JSON_LIMIT))):
        raise PrivacyError("Missing or changed privacy receipt")
    return files


def stage_native(packages, smoke, publish, seal, target, source, root, scratch):
    directory(packages)
    name = package.TARGETS[target][2]
    files = {name: bounded_read(packages / name),
             name + ".sha256": bounded_read(packages / (name + ".sha256"), 256)}
    if files[name + ".sha256"] != checksum(name, files[name]):
        raise PrivacyError("Native archive checksum mismatch")
    binary, asset_count = inspect_distribution(files[name], target, source, root, scratch)
    proof = parse(bounded_read(smoke, JSON_LIMIT))
    if (proof.get("result"), proof.get("sourceCommit"), proof.get("archiveSha256"),
            proof.get("binarySha256"), proof.get("verifiedEmbeddedAssets")) != (
            "PASS", source.commit, digest(files[name]), binary, asset_count):
        raise PrivacyError("Missing matching native smoke verification")
    stage(files, publish, seal, source.commit, target)


def checksum(name, content):
    return f"{digest(content)}  {name}\n".encode("ascii")


def received_inventory(downloaded, source):
    directory(downloaded)
    expected = {f"candidate-native-{target}-{source}" for target in package.TARGETS}
    if {item.name for item in downloaded.iterdir()} != expected:
        raise PrivacyError("Unexpected native artifact set")
    files = {}
    for target in package.TARGETS:
        native = read_screened(downloaded / f"candidate-native-{target}-{source}", source, target)
        files.update(native)
    return files


def stage_inventory(assembled, screened, publish, seal, source):
    files = flat_files(assembled, names_for("inventory"))
    checksums = b"".join(checksum(name, screened[name]) for name in sorted(screened) if not name.endswith(".sha256"))
    if files[CHECKSUMS] != checksums or any(files[name] != data for name, data in screened.items()):
        raise PrivacyError("Assembled files differ from screened archives")
    stage(files, publish, seal, source, "inventory")
