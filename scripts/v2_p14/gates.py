import argparse
import hashlib
import json
from pathlib import Path
import re

GATES = {
    "G01": "Clean integrated source baseline", "G02": "Full Rust suite", "G03": "Full frontend suite", "G04": "Security regressions",
    "G05": "Windows x86_64 native qualification", "G06": "Linux x86_64 native qualification", "G07": "macOS ARM64 native qualification", "G08": "macOS x86_64 native qualification",
    "G09": "Production packages", "G10": "Fresh installation", "G11": "Upgrade and migration", "G12": "V1 compatibility",
    "G13": "Genuine candidate coding-agent journey", "G14": "Parallel scheduling and recovery", "G15": "Suspension and manual resume", "G16": "Memory correctness",
    "G17": "Git integrity", "G18": "GitHub authorization and live delivery qualification", "G19": "Accessibility", "G20": "Release performance",
    "G21": "Dependency and security audit", "G22": "Artifact integrity", "G23": "Exact-commit remote CI", "G24": "Release documentation", "G25": "No critical/high blockers",
}
STATUSES = {"PASS", "FAIL", "BLOCKED", "UNVERIFIED", "NOT_APPLICABLE"}


def evaluate(data, evidence_root):
    source = data.get("sourceCommit", "")
    if not re.fullmatch("[0-9a-f]{40}", source):
        raise ValueError("Exact source commit required")
    records = data.get("gates", [])
    if len(records) != len(GATES) or {record.get("id") for record in records} != set(GATES):
        raise ValueError("All 25 mandatory gates must appear exactly once")
    root = evidence_root.resolve()
    for record in records:
        if record.get("status") not in STATUSES:
            raise ValueError("Unknown gate status")
        evidence = record.get("evidence", [])
        if record["status"] == "PASS" and not evidence:
            raise ValueError("PASS requires evidence")
        current = False
        for item in evidence:
            path = (root / item["path"]).resolve()
            if not path.is_relative_to(root) or not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != item["sha256"]:
                raise ValueError("Evidence path or hash mismatch")
            current |= item.get("sourceCommit") == source and item.get("scope") == "candidate" and item.get("result") == "PASS"
        if record["status"] == "PASS" and not current:
            raise ValueError("Historical evidence alone cannot qualify the candidate")
    counts = {status: sum(record["status"] == status for record in records) for status in sorted(STATUSES)}
    ci = data.get("remoteCI", {})
    remote_verified = ci.get("sourceCommit") == source and ci.get("repository") == "OmegaMc1331/agentic-software-factory" and all(ci.get("jobs", {}).get(target, {}).get("conclusion") == "success" and ci["jobs"][target].get("runId") and ci["jobs"][target].get("jobId") for target in ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-apple-darwin"])
    authorized = data.get("releaseAuthorization", {}).get("explicit") is True and data["releaseAuthorization"].get("sourceCommit") == source
    decision = "GO" if counts["PASS"] == len(GATES) and remote_verified and authorized else "NO-GO"
    return {"schemaVersion": 1, "sourceCommit": source, "decision": decision, "counts": counts, "remoteCIMatches": bool(remote_verified), "explicitReleaseAuthorization": bool(authorized), "publicationPerformed": False, "limitation": "This evaluates recorded evidence. It does not grant publication authority or replace remote API/attestation verification."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--evidence-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = evaluate(json.loads(args.input.read_text(encoding="utf-8")), args.evidence_root)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))
