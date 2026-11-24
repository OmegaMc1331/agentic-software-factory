import json
import pathlib
import statistics


ROOT = pathlib.Path(__file__).resolve().parents[3]
STATE = ROOT / ".factory/v2/p01"


def median(rows, key):
    return round(statistics.median(row[key] for row in rows), 2)


def main():
    measured = json.loads((STATE / "benchmarks.json").read_text())["rows"]
    services = json.loads((STATE / "service-benchmarks.json").read_text())["rows"]
    summary = {"schemaVersion": 1, "classification": "derived medians of retained raw measurements, n=3 per row", "interface": [], "services": []}
    keys = ["launchMs", "newBrowserProjectReadyMs", "coldProcessTotalMs", "openFileMs", "tabSwitchMs", "splitMs", "largeFileMs", "inputToTwoFramesMs", "warmReloadMs"]
    for candidate in ("monaco", "codemirror"):
        for project in ("small", "medium", "large"):
            rows = [row for row in measured if row["candidate"] == candidate and row["project"] == project]
            row = {"candidate": candidate, "project": project, **{key: median(rows, key) for key in keys}}
            row["heapUsedBytes"] = statistics.median(item["heap"]["used"] for item in rows)
            row["ownedWorkingSetBytes"] = statistics.median(sum(process["WorkingSet64"] for process in item["processMemory"]) for item in rows)
            row["ownedPrivateBytes"] = statistics.median(sum(process["PrivateMemorySize64"] for process in item["processMemory"]) for item in rows)
            row["initialEncodedResourceBytes"] = statistics.median(sum(resource["encodedBodySize"] for resource in item["resources"]) for item in rows)
            summary["interface"].append(row)
        rows = [row for row in services if row["candidate"] == candidate]
        row = {"candidate": candidate, "lspReadyMs": round(statistics.median(item["lsp"]["readyMs"] for item in rows), 2), "firstDiagnosticMs": median(rows, "firstDiagnosticMs"), "adapterReadyMs": round(statistics.median(item["dap"]["adapterReadyMs"] for item in rows), 2), "breakpointMs": round(statistics.median(item["dap"]["breakpointMs"] for item in rows), 2), "lspTreeWorkingSetBytes": statistics.median(sum(process["WorkingSet64"] for process in item["lspProcessTreeMemory"]) for item in rows)}
        row["extensionActivationMs"] = round(statistics.median(item["extension"]["elapsedMs"] for item in rows), 2) if candidate == "codemirror" else None
        summary["services"].append(row)
    (STATE / "benchmark-summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
