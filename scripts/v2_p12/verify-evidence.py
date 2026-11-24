import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parents[2] / "docs/v2/evidence/p12"
manifest = json.loads((root / "manifest.json").read_text(encoding="utf8"))
assert manifest["schema"] == 1 and manifest["phase"] == "V2-P12"
listed = set()
for record in manifest["files"]:
    name = record["path"]
    target = (root / name).resolve()
    assert target.is_relative_to(root.resolve()), name
    assert name not in listed, name
    listed.add(name)
    data = target.read_bytes()
    assert len(data) == record["bytes"], name
    assert hashlib.sha256(data).hexdigest() == record["sha256"], name
actual = {str(p.relative_to(root)).replace("\\", "/") for p in root.rglob("*") if p.is_file() and p.name != "manifest.json"}
assert actual == listed, "Unindexed or missing evidence files"
summary = json.loads((root / "validation-summary.json").read_text(encoding="utf8"))
assert all(v["status"] == "PASS" and v["sourceUnchanged"] for v in summary["validation"])
assert summary["browserCount"] == 104 and all(v["allPassed"] for v in summary["browserRegressions"])
build = manifest["currentRuntimeBuild"]
for validation in summary["validation"]:
    if validation["mode"] in {"focused", "api_parallel", "refs", "rust", "parallel", "embedded", "tools"}:
        assert validation["sourceFingerprint"] == build["sourceFingerprint"], validation["mode"]
fixed = json.loads((root / "security/ref-alias-fixed.json").read_text(encoding="utf8"))
assert fixed["status"] == "GUARD_ENFORCED" and fixed["binarySha256"] == build["sha256"]
assert fixed["safeCreation"]["status"] == 200 and fixed["safeReturn"]["status"] == 200
assert fixed["caseAlias"]["status"] == 400 and fixed["ownedExact"]["status"] == 403
assert fixed["unicodeOwned"]["status"] == 403 and fixed["finalBranch"] == "main"
assert fixed["finalHead"] == fixed["initialHead"] and fixed["cleanup"]["ownedServerExited"]
for stage in manifest["stages"]:
    assert stage["status"] == "PASS", stage["stage"]
manual = json.loads((root / "manual-browser/results.json").read_text(encoding="utf8"))
assert manual["binarySha256"] == build["sha256"] and len(manual["tests"]) == 6
assert manual["publicationRequests"] == [] and manual["cleanup"]["remoteWrites"] == 0
assert manifest["liveGitHubWriteAcceptance"] == "NOT QUALIFIED - NO AUTHORIZED REMOTE MUTATION"
print(json.dumps({"status": "PASS", "files": len(listed), "bytes": sum(r["bytes"] for r in manifest["files"]), "sourceCommit": build["sourceCommit"], "binarySha256": build["sha256"], "retainedBrowserScenarios": summary["browserCount"], "liveGitHubWrites": "NOT QUALIFIED - NO AUTHORIZED REMOTE MUTATION"}))
