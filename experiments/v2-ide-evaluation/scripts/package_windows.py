import hashlib
import json
import pathlib
import shutil
import subprocess
import urllib.request
import zipfile
from license_inventory import inventory


ROOT = pathlib.Path(__file__).resolve().parents[3]
STATE = ROOT / ".factory/v2/p01"
EXPERIMENT = ROOT / "experiments/v2-ide-evaluation"
PACKAGE = STATE / "package-windows-x64"
PYTHON_SHA = "90905e64a840557d8c1bc4f5b74db3d7ee85e3ffb217611cac1c7813acea4159"


def download(url, name, expected=None):
    target = STATE / "downloads" / name
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        urllib.request.urlretrieve(url, target)
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    if expected and digest != expected:
        raise RuntimeError(f"SHA-256 mismatch for {name}")
    return target, {"url": url, "sha256": digest, "bytes": target.stat().st_size}


def main():
    PACKAGE.mkdir(parents=True, exist_ok=True)
    marker = PACKAGE / ".asf-p01-package"
    if any(PACKAGE.iterdir()) and not marker.exists():
        raise RuntimeError("refusing to write into an unowned package directory")
    marker.write_text("ASF_P01_PACKAGE_V1", encoding="utf-8")
    shutil.copy2(ROOT / "target/release/asf-p01-ide-probe.exe", PACKAGE / "asf-p01-ide-probe.exe")
    manifest = {"schemaVersion": 1, "target": "x86_64-pc-windows-msvc", "sources": []}
    node_version = subprocess.check_output(["node", "--version"], text=True).strip()
    if node_version != "v24.22.1":
        raise RuntimeError("this evidence recipe pins Node v24.22.1")
    checksums, check_evidence = download(f"https://nodejs.org/dist/{node_version}/SHASUMS256.txt", "node-SHASUMS256.txt")
    filename = f"node-{node_version}-win-x64.zip"
    expected = next(line.split()[0] for line in checksums.read_text().splitlines() if line.split()[-1] == filename)
    archive, node_evidence = download(f"https://nodejs.org/dist/{node_version}/{filename}", filename, expected)
    manifest["sources"].extend([check_evidence, node_evidence])
    runtime = PACKAGE / "runtime"
    runtime.mkdir(exist_ok=True)
    with zipfile.ZipFile(archive) as zipped:
        for source, destination in ((f"node-{node_version}-win-x64/node.exe", "node.exe"), (f"node-{node_version}-win-x64/LICENSE", "NODE-LICENSE.txt")):
            (runtime / destination).write_bytes(zipped.read(source))
    python_archive, python_evidence = download("https://www.python.org/ftp/python/3.14.10/python-3.14.10-embed-amd64.zip", "python-3.14.10-embed-amd64.zip", PYTHON_SHA)
    manifest["sources"].append(python_evidence)
    python_home = runtime / "python"
    python_home.mkdir(exist_ok=True)
    with zipfile.ZipFile(python_archive) as zipped:
        for entry in zipped.infolist():
            resolved = (python_home / entry.filename).resolve()
            if not resolved.is_relative_to(python_home.resolve()):
                raise RuntimeError("unsafe archive member")
        zipped.extractall(python_home)
    (python_home / "python314._pth").write_text("python314.zip\n.\nLib/site-packages\nimport site\n", encoding="utf-8")
    source_site = STATE / "python/Lib/site-packages"
    for source in source_site.glob("debugpy*"):
        if source.is_dir():
            shutil.copytree(source, python_home / "Lib/site-packages" / source.name, dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    tools = PACKAGE / "tools/node_modules"
    for name in ("typescript-language-server", "typescript", "pyright"):
        shutil.copytree(EXPERIMENT / "node_modules" / name, tools / name, dirs_exist_ok=True)
    shutil.copy2(EXPERIMENT / "scripts/terminal_probe.py", PACKAGE / "terminal_probe.py")
    shutil.copy2(EXPERIMENT / "scripts/launch_package.ps1", PACKAGE / "launch.ps1")
    notices = PACKAGE / "licenses"
    notices.mkdir(exist_ok=True)
    license_rows = inventory()
    (notices / "npm-license-inventory.json").write_text(json.dumps(license_rows, indent=2), encoding="utf-8")
    for package in license_rows:
        for notice in package["notices"]:
            filename = EXPERIMENT / notice["path"]
            destination_name = package["name"].replace("/", "_").replace("@", "") + "-" + package["version"] + "-" + filename.name
            shutil.copy2(filename, notices / destination_name)
    shutil.copy2(ROOT / "LICENSE", notices / "ASF-LICENSE")
    subprocess.run([str(python_home / "python.exe"), "-c", "import debugpy,sys; print(sys.version); print(debugpy.__version__)"], check=True)
    manifest["files"] = [{"path": str(file.relative_to(PACKAGE)).replace("\\", "/"), "bytes": file.stat().st_size, "sha256": hashlib.sha256(file.read_bytes()).hexdigest()} for file in PACKAGE.rglob("*") if file.is_file() and file.name != "manifest.json"]
    (PACKAGE / "manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    archive = pathlib.Path(shutil.make_archive(str(STATE / "asf-p01-windows-x64"), "zip", PACKAGE))
    evidence = {"schemaVersion": 1, "package": str(archive.relative_to(ROOT)), "bytes": archive.stat().st_size, "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(), "sources": manifest["sources"], "fileCount": len(manifest["files"]), "uncompressedBytes": sum(item["bytes"] for item in manifest["files"]), "scope": "local representative packaging experiment, not an ASF release"}
    (STATE / "packaging-build.json").write_text(json.dumps(evidence, indent=2), encoding="utf-8")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    main()
