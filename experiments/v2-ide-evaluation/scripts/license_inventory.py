import hashlib
import json
import pathlib


ROOT = pathlib.Path(__file__).resolve().parents[3]
EXPERIMENT = ROOT / "experiments/v2-ide-evaluation"
STATE = ROOT / ".factory/v2/p01"


def inventory():
    lock = json.loads((EXPERIMENT / "package-lock.json").read_text())
    result = []
    for name, entry in lock["packages"].items():
        if not name:
            continue
        directory = EXPERIMENT / name
        metadata = json.loads((directory / "package.json").read_text(encoding="utf-8")) if (directory / "package.json").exists() else {}
        notices = [{"path": str(file.relative_to(EXPERIMENT)).replace("\\", "/"), "sha256": hashlib.sha256(file.read_bytes()).hexdigest()} for file in directory.iterdir() if file.is_file() and file.name.lower().startswith(("license", "licence", "notice", "thirdparty", "copyright"))] if directory.exists() else []
        result.append({"name": metadata.get("name", name.split("node_modules/")[-1]), "version": entry.get("version"), "license": metadata.get("license", entry.get("license")), "dev": entry.get("dev", False), "integrity": entry.get("integrity"), "resolved": entry.get("resolved"), "installedOnHost": bool(metadata), "notices": notices})
    return result


def main():
    rows = inventory()
    (STATE / "license-inventory.json").write_text(json.dumps({"schemaVersion": 1, "scope": "experimental npm lockfile and installed notice paths; not a full shipping Rust/native SBOM", "packages": rows}, indent=2), encoding="utf-8")
    print(json.dumps({"packages": len(rows), "unknownLicenses": [row["name"] for row in rows if not row["license"]]}))


if __name__ == "__main__":
    main()
