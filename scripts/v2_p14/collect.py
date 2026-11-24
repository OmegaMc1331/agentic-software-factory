import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re

GROUPS = {"baseline", "builds", "ci", "windows", "linux", "macos-arm64", "macos-x86_64", "packaging", "migration", "security", "dependencies", "performance", "accessibility", "browsers", "real-agents", "github", "screenshots", "release", "frontend", "contracts"}
EXCLUDED = {"tools", "advisory-db", "node_modules", "packages", "artifacts", "unpacked", "source", "clean-checkout"}
EXTENSIONS = {".json", ".log", ".txt", ".md", ".png"}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--index-only", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch("[0-9a-f]{40}", args.candidate):
        raise ValueError("Exact candidate source required")
    args.destination.mkdir(parents=True, exist_ok=True)
    copied, omitted = [], []
    secrets = [value for key, value in os.environ.items() if re.search("TOKEN|SECRET|PASSWORD|API_KEY", key, re.I) and len(value) >= 8]
    if not args.index_only:
        if args.source is None:
            parser.error("--source is required unless indexing existing evidence")
        for file in sorted(args.source.rglob("*")):
            relative = file.relative_to(args.source)
            if not file.is_file() or relative.parts[0] not in GROUPS or set(relative.parts) & EXCLUDED or file.suffix.lower() not in EXTENSIONS:
                continue
            if file.stat().st_size > 2_000_000:
                omitted.append({"path": relative.as_posix(), "bytes": file.stat().st_size, "reason": "Size bound; retained locally"})
                continue
            raw = file.read_bytes()
            data = raw
            if file.suffix != ".png":
                encoding = "utf-16" if raw.startswith((b"\xff\xfe", b"\xfe\xff")) else "utf-8-sig"
                text = raw.decode(encoding, errors="replace")
                for secret in secrets:
                    text = text.replace(secret, "[REDACTED]")
                data = text.encode("utf-8")
            target = args.destination / relative
            if target.exists() and target.read_bytes() != data:
                raise ValueError("Preserve changed evidence in a new revision: " + str(target))
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            copied.append({"path": relative.as_posix(), "rawSha256": digest(raw), "exportSha256": digest(data), "transformed": raw != data})
        (args.destination / "export.json").write_text(json.dumps({"schemaVersion": 1, "copies": copied, "omitted": omitted, "redaction": "Known inherited credential values only; no perfect secret detection claim."}, indent=2) + "\n", encoding="utf-8")
    entries = [{"path": file.relative_to(args.destination).as_posix(), "bytes": file.stat().st_size, "sha256": digest(file.read_bytes())} for file in sorted(args.destination.rglob("*")) if file.is_file() and file.name != "manifest.json"]
    manifest = {"schemaVersion": 1, "candidateSourceCommit": args.candidate, "timestamp": datetime.datetime.now(datetime.UTC).isoformat(), "files": entries, "limitations": ["Each execution receipt retains its own observed source/environment/result. Inventory inclusion does not mean PASS or candidate-equivalent execution.", "Large artifacts and private tool data are retained outside source control."]}
    (args.destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"files": len(entries), "omitted": len(omitted)}))


if __name__ == "__main__":
    main()
