import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tarfile
import urllib.parse
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
AUDIT = {
    ("Windows", "AMD64"): ("cargo-audit-x86_64-pc-windows-msvc-v0.23.0.zip", "aafeb61206730029601a72e669a12231f8674427e6248d02f6cb382cb36d56b4"),
    ("Linux", "x86_64"): ("cargo-audit-x86_64-unknown-linux-gnu-v0.23.0.tgz", "b77993532b253fcbf550996d7716465ec3d734d4e74e7f87bfb65a19add0fb59"),
}


def supplemental_notices(identity, directory, supplement_root, supplements):
    notices = []
    for entry in supplements:
        if entry["purl"] != identity:
            continue
        file = supplement_root / entry["file"]
        if file.parent.resolve() != supplement_root.resolve() or hashlib.sha256(file.read_bytes()).hexdigest() != entry["sha256"]:
            raise ValueError("Supplemental upstream license integrity failed")
        vcs = json.loads((directory / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
        if vcs["git"]["sha1"] != entry["revision"]:
            raise ValueError("Supplemental license does not match published Cargo source identity")
        notices.append(f"\n=== {identity} / {entry['url']} ===\n" + file.read_text(encoding="utf-8"))
    return notices


def inventory(output):
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--format-version", "1", "--all-features"], cwd=ROOT))
    components, notices, missing = [], [], []
    supplement_root = ROOT / "scripts/v2_p14/licenses"
    supplements = json.loads((supplement_root / "manifest.json").read_text(encoding="utf-8"))["licenses"]
    for package in metadata["packages"]:
        if package["source"] is None:
            continue
        identity = f"pkg:cargo/{package['name']}@{package['version']}"
        component = {"type": "library", "bom-ref": identity, "name": package["name"], "version": package["version"], "purl": identity, "properties": [{"name": "asf:inventory-scope", "value": "locked Rust graph including build/dev/optional targets"}]}
        if package.get("license"):
            expression = package["license"].replace("MIT/Apache-2.0", "MIT OR Apache-2.0")
            component["licenses"] = [{"expression": expression}]
        directory = Path(package["manifest_path"]).parent
        candidates = [file for file in directory.iterdir() if file.is_file() and file.name.lower().startswith(("license", "licence", "copying", "notice"))]
        if package.get("license_file"):
            candidate = directory / package["license_file"]
            if candidate.is_file() and candidate.resolve().is_relative_to(directory.resolve()):
                candidates.append(candidate)
        supplied = supplemental_notices(identity, directory, supplement_root, supplements)
        notices.extend(supplied)
        if not candidates and not supplied:
            missing.append(identity)
        for file in sorted(set(candidates)):
            notices.append(f"\n=== {identity} / {file.name} ===\n" + file.read_text(encoding="utf-8", errors="replace"))
        if any("custom-build" in target["kind"] for target in package["targets"]):
            component["properties"].append({"name": "asf:build-script", "value": "present"})
        components.append(component)
    lock = json.loads((ROOT / "apps/dashboard/package-lock.json").read_text(encoding="utf-8"))
    for name, package in lock["packages"].items():
        if not name or "node_modules/" not in name:
            continue
        module_name = name.rsplit("node_modules/", 1)[1]
        identity = f"pkg:npm/{urllib.parse.quote(module_name, safe='/')}@{package['version']}"
        component = {"type": "library", "bom-ref": identity + ":" + name, "name": module_name, "version": package["version"], "purl": identity, "scope": "optional" if package.get("dev") else "required"}
        if package.get("license"):
            component["licenses"] = [{"license": {"name": package["license"]}}]
        directory = ROOT / "apps/dashboard" / name
        candidates = [file for file in directory.iterdir() if file.is_file() and file.name.lower().startswith(("license", "licence", "copying", "notice"))] if directory.exists() else []
        if not candidates and not package.get("dev"):
            missing.append(identity)
        for file in sorted(candidates):
            notices.append(f"\n=== {identity} / {file.name} ===\n" + file.read_text(encoding="utf-8", errors="replace"))
        components.append(component)
    for file in sorted((ROOT / "apps/dashboard/public/licenses").glob("*")):
        if file.is_file():
            notices.append(f"\n=== Embedded font / {file.name} ===\n" + file.read_text(encoding="utf-8"))
    bom = {"bomFormat": "CycloneDX", "specVersion": "1.6", "version": 1, "components": components}
    (output / "sbom.cdx.json").write_text(json.dumps(bom, indent=2) + "\n", encoding="utf-8")
    (output / "THIRD-PARTY-NOTICES.txt").write_text("Locked dependency inventory; includes development and optional dependencies.\n" + "\n".join(notices), encoding="utf-8")
    (output / "license-review.json").write_text(json.dumps({"schemaVersion": 1, "componentCount": len(components), "missingLicenseText": missing, "limitation": "License inventory is not legal approval or an assertion that every listed dependency is linked into the binary."}, indent=2) + "\n", encoding="utf-8")
    return bom


def auditor(output):
    name, expected = AUDIT[(platform.system(), platform.machine())]
    tools = output / "tools"
    tools.mkdir(parents=True, exist_ok=True)
    archive = tools / name
    if not archive.exists():
        with urllib.request.urlopen("https://github.com/RustSec/rustsec/releases/download/cargo-audit/v0.23.0/" + name, timeout=90) as response:
            data = response.read(100_000_001)
        if len(data) > 100_000_000 or hashlib.sha256(data).hexdigest() != expected:
            raise ValueError("RustSec audit tool archive integrity failed")
        archive.write_bytes(data)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
        raise ValueError("Cached RustSec audit tool archive integrity failed")
    binary = tools / ("cargo-audit.exe" if os.name == "nt" else "cargo-audit")
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as package:
            members = [member for member in package.infolist() if Path(member.filename).name == binary.name and not member.is_dir()]
            if len(members) != 1:
                raise ValueError("Unexpected RustSec archive inventory")
            binary.write_bytes(package.read(members[0]))
    else:
        with tarfile.open(archive) as package:
            members = [member for member in package.getmembers() if Path(member.name).name == binary.name and member.isfile()]
            if len(members) != 1:
                raise ValueError("Unexpected RustSec archive inventory")
            binary.write_bytes(package.extractfile(members[0]).read())
    binary.chmod(0o755)
    return binary.resolve()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--inventory-only", action="store_true")
    args = parser.parse_args()
    inventory(args.output)
    if args.inventory_only:
        return
    binary = auditor(args.output)
    database = (args.output / "advisory-db").resolve()
    result = subprocess.run([str(binary), "audit", "--json", "--db", str(database), "--file", str(ROOT / "Cargo.lock")], cwd=ROOT, capture_output=True, check=False)
    (args.output / "cargo-audit.json").write_bytes(result.stdout)
    (args.output / "cargo-audit.stderr.txt").write_bytes(result.stderr)
    (args.output / "audit-execution.json").write_text(json.dumps({"schemaVersion": 1, "sourceCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "timestamp": datetime.datetime.now(datetime.UTC).isoformat(), "tool": "cargo-audit 0.23.0", "archiveDigest": AUDIT[(platform.system(), platform.machine())][1], "exit": result.returncode, "result": "PASS" if result.returncode == 0 else "FAIL", "advisoryDatabaseCommit": subprocess.run(["git", "-C", str(database), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()}, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
