import gzip
import hashlib
import json
import platform
import sqlite3
import statistics
from datetime import datetime
from pathlib import Path

root = Path(__file__).resolve().parents[2]
local = root / ".factory/v2/p10"
native = json.loads((local / "native/results.json").read_text(encoding="utf8"))
assert native["status"] == "PASS"


def millis(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp() * 1000


def summarize(samples):
    return {"samples": len(samples), "medianMs": statistics.median(samples), "maxMs": max(samples), "valuesMs": samples}


result = {"schema": 1, "platform": platform.platform(), "method": "Derived from retained observed API/event/process measurements; no new agent requests", "presence": [], "missions": {}, "modelInvocations": native["modelInvocationCount"], "nativeSourceCommit": native["sourceCommit"], "nativeBinarySha256": native["binarySha256"], "providerTokenAndCostAccounting": "CLI-reported counts retained separately; interrupted usage and monetary cost are unknown"}
for clients in [1, 5, 20]:
    for operation in ["register", "heartbeat"]:
        result["presence"].append({"clients": clients, "operation": operation, **summarize([s["ms"] for s in native["presencePerformance"] if s["clients"] == clients and s["operation"] == operation])})
timeline = native["timeline"]
closed = next(t for t in timeline if t["event"] == "browser.final_closed")
suspended = next(t for t in timeline if t["event"] == "mission.absence_suspended")
result["finalClientClosureToObservedSuspension"] = {"samples": 1, "ms": millis(suspended["at"]) - millis(closed["at"]), "leaseSeconds": 6, "graceSeconds": 3, "productionDefaults": {"leaseSeconds": 180, "graceSeconds": 60}}
result["resumeUiIncludingDuplicateAcknowledgement"] = summarize([t["requestToAcknowledgementMs"] for t in timeline if t["event"] == "mission.resume_acknowledged"])
result["resumeUiMeasurementNote"] = "Includes explicit UI request, auth refresh and duplicate acknowledgement; not isolated endpoint latency"
for name, project in native["projects"].items():
    events = project["events"]["events"]
    suspension = []
    resumes = []
    for checkpoint in (e for e in events if e["event_type"] == "mission.checkpoint_created"):
        generation = checkpoint["payload"]["generation"]
        earlier = [e for e in events if e["payload"].get("generation") == generation and e["id"] < checkpoint["id"]]
        quiescing = next(e for e in earlier if e["event_type"] == "mission.quiescing")
        request = next((e for e in earlier if e["event_type"] == "mission.suspend_requested"), None)
        row = {"generation": generation, "quiescingToCheckpointMs": millis(checkpoint["created_at"]) - millis(quiescing["created_at"])}
        if request:
            row["requestToQuiescingMs"] = millis(quiescing["created_at"]) - millis(request["created_at"])
            exits = [s["outputMeta"]["processFinishedUnixMs"] for s in project["sessions"] if s["status"] == "interrupted" and s["outputMeta"].get("processFinishedUnixMs") and s["outputMeta"]["processFinishedUnixMs"] >= millis(request["created_at"]) - 10]
            if exits:
                row["requestToObservedOwnedProcessExitMs"] = min(exits) - millis(request["created_at"])
        suspension.append(row)
    for resumed in (e for e in events if e["event_type"] == "mission.resumed"):
        request = next(e for e in events if e["event_type"] == "mission.resume_requested" and e["payload"]["generation"] == resumed["payload"]["generation"])
        starts = [s["outputMeta"]["processStartedUnixMs"] for s in project["sessions"] if s["role"] == "worker" and s["outputMeta"].get("processStartedUnixMs", 0) >= millis(resumed["created_at"])]
        resumes.append({"generation": resumed["payload"]["generation"], "validationAndCommitMs": millis(resumed["created_at"]) - millis(request["created_at"]), "commitToNextWorkerStartMs": min(starts) - millis(resumed["created_at"]) if starts else None})
    dbpath = Path(project["root"]) / ".factory/db.sqlite3"
    db = sqlite3.connect("file:" + str(dbpath) + "?mode=ro", uri=True)
    count, payload = db.execute("SELECT count(*), COALESCE(SUM(length(CAST(payload AS BLOB))),0) FROM mission_checkpoints").fetchone()
    result["missions"][name] = {"suspensions": suspension, "resumes": resumes, "retainedDatabaseBytes": dbpath.stat().st_size, "checkpointCount": count, "checkpointPayloadBytes": payload, "retainedAgentSessions": len(project["sessions"]), "note": "Database size includes all agent output/history; it is not solely lifecycle overhead"}
    db.close()
result["serverMemory"] = {"samples": len(native["memorySamples"]), "peakObservedWorkingSetBytes": max(s.get("workingSet", 0) for s in native["memorySamples"]), "peakObservedPrivateBytes": max(s.get("privateBytes", 0) for s in native["memorySamples"]), "method": "Owned server at recorded milestones; excludes browser, external CLI and test children"}
assets = [{"path": "assets/" + file.name, "bytes": file.stat().st_size, "gzipBytes": len(gzip.compress(file.read_bytes(), mtime=0)), "sha256": hashlib.sha256(file.read_bytes()).hexdigest()} for file in sorted((root / "apps/dashboard/dist/assets").iterdir()) if file.is_file()]
baseline = json.loads((root / "docs/v2/evidence/p09/browser-replay.json").read_text(encoding="utf8"))["assets"]
result["bundleDelta"] = {extension: sum(a["bytes"] for a in assets if a["path"].endswith(extension)) - sum(a["bytes"] for a in baseline if a["path"].endswith(extension)) for extension in [".js", ".css"]}
result["limitations"] = ["One real combined acceptance journey with two projects; timings are observations, not latency guarantees", "Benchmark leases are simultaneous but requests are sequential; this is not a throughput saturation benchmark", "Native Linux/macOS and frozen-page/sleep-wake behavior remain unqualified", "Only retained completed-session CLI token fields are available; interrupted-session usage, provider billing and monetary cost are not inferred"]
final_path = local / "controlled-final/results.json"
if final_path.exists():
    final = json.loads(final_path.read_text(encoding="utf8"))
    assert final["status"] == "PASS"
    final_closed = next(t for t in final["timeline"] if t["event"] == "browser.final_closed")
    final_suspended = next(t for t in final["timeline"] if t["event"] == "mission.absence_suspended")
    result["finalBuildControlled"] = {"binarySha256": final["binarySha256"], "sourceCommit": final["sourceCommit"], "modelInvocations": 0, "closureToObservedSuspensionMs": millis(final_suspended["at"]) - millis(final_closed["at"]), "presenceSamples": final["presencePerformance"], "memorySamples": final["memorySamples"], "note": "Controlled process journey validates post-native guards; it is not model execution throughput evidence"}
(local / "frontend-assets.json").write_text(json.dumps({"schema": 1, "baseline": "docs/v2/evidence/p09/browser-replay.json", "assets": assets}, indent=2) + "\n", encoding="utf8")
(local / "performance.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf8")
print(json.dumps({"presenceSamples": len(native["presencePerformance"]), "bundleDelta": result["bundleDelta"], "observedClosureMs": result["finalClientClosureToObservedSuspension"]["ms"]}))
