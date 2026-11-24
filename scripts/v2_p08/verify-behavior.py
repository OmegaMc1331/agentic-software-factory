import importlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
checks = 0


def check(value):
    global checks
    assert value
    checks += 1


def verify(folder, scenario):
    sys.path.insert(0, folder)
    if scenario == "Parallel":
        validation = importlib.import_module("src.validation")
        events = importlib.import_module("src.events")
        calculator = importlib.import_module("src.calculator")
        strings = importlib.import_module("src.strings")
        valid = {"id": 7, "name": "  Ada LOVELACE  ", "extra": [1, 2]}
        check(validation.validate_request(valid) == valid)
        for invalid in [None, [], {"id": True, "name": "Ada"}, {"id": 0, "name": "Ada"}, {"id": -1, "name": "Ada"}, {"id": "1", "name": "Ada"}, {"id": 1, "name": ""}, {"id": 1, "name": 3}]:
            try:
                validation.validate_request(invalid)
            except ValueError:
                check(True)
            else:
                check(False)
        for value in range(-20, 21):
            for divisor in range(1, 6):
                check(calculator.ceil_div(value, divisor) == -(-value // divisor))
        for divisor in [0, -1]:
            try:
                calculator.ceil_div(7, divisor)
            except ValueError:
                check(True)
            else:
                check(False)
        for value in ["", "  Ada LOVELACE  ", "hello\tworld", " ONE\nTWO  three "]:
            check(strings.slug(value) == "-".join(value.lower().split()))
        payload = {"id": valid["id"], "name": strings.slug(valid["name"]), "groups": calculator.ceil_div(valid["id"], 3)}
        encoded = events.render_event("accepted", payload)
        check(json.loads(encoded) == {"event": "accepted", "payload": payload})
        check(events.render_event("accepted", payload) == encoded)
    elif scenario == "Conflict":
        flags = importlib.import_module("src.features").flags()
        for name in ["base", "validation", "logging"]:
            check(flags[name] is True)
    else:
        raise ValueError("Only completed real scenarios may be verified")
    print(json.dumps({"scenario": scenario, "checks": checks, "status": "PASS"}))


if len(sys.argv) > 1:
    verify(sys.argv[1], sys.argv[2])
else:
    results = []
    for source in ["acceptance", "acceptance-parallel-r2"]:
        record = ROOT / ".factory/v2/p08" / source / "results.json"
        acceptance = json.loads(record.read_text(encoding="utf8"))
        for name, item in acceptance["scenarios"].items():
            if item.get("independent", {}).get("exit") != 0:
                continue
            folder = item["independent"]["cwd"]
            start = time.perf_counter()
            run = subprocess.run([sys.executable, "-B", str(Path(__file__).resolve()), folder, name], capture_output=True, encoding="utf8", env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}, check=False)
            results.append({"source": source, "scenario": name, "integration_sha": item["mission"]["integration_sha"], "checkout": folder, "exit": run.returncode, "stdout": run.stdout, "stderr": run.stderr, "ms": (time.perf_counter() - start) * 1000})
    output = ROOT / ".factory/v2/p08/independent-behavior.json"
    output.write_text(json.dumps(results, indent=2) + "\n", encoding="utf8")
    print(json.dumps(results, ensure_ascii=True))
    raise SystemExit(0 if all(result["exit"] == 0 for result in results) else 1)
