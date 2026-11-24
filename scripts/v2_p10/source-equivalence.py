import json
import sqlite3
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[2]
local = root / ".factory/v2/p10"
native = json.loads((local / "native/results.json").read_text(encoding="utf8"))
assert native["status"] == "PASS"


def git(*arguments):
    return subprocess.check_output(["git", *arguments], cwd=root, encoding="utf8").strip()


def tree(commit):
    lines = git("ls-tree", "-r", "--format=%(objectname)\t%(path)", commit, "--", "Cargo.toml", "Cargo.lock", "crates", "apps/dashboard/src").splitlines()
    return {line.split("\t", 1)[1]: line.split("\t", 1)[0] for line in lines}


head = git("rev-parse", "HEAD")
original = tree(native["sourceCommit"])
current = tree(head)
paths = sorted(file for file in original if file.startswith("crates/") and ("/src/" in file or file.endswith("Cargo.toml")) or file in {"Cargo.toml", "Cargo.lock"})
files = [{"file": file, "nativeBlob": original[file], "finalBlob": current.get(file), "equal": original[file] == current.get(file)} for file in paths]
changed = {file["file"] for file in files if not file["equal"]}
assert changed == {"crates/factory-db/src/lifecycle.rs", "crates/factory-api/src/lifecycle.rs"}, changed
uncommitted = git("diff", "--name-only", "HEAD", "--", "crates", "apps/dashboard/src")
assert not uncommitted, uncommitted
counts = {}
for name, project in native["projects"].items():
    connection = sqlite3.connect("file:" + str(Path(project["root"]) / ".factory/db.sqlite3") + "?mode=ro", uri=True)
    counts[name] = connection.execute("SELECT count(*) FROM lifecycle_commands WHERE run_id=?", (project["runId"],)).fetchone()[0]
    connection.close()
value = {"schema": 1, "nativeSourceCommit": native["sourceCommit"], "finalImplementationCommit": head, "nativeBinarySha256": native["binarySha256"], "verifiedExecutionFiles": files, "postAcceptanceRuntimeDifferences": {"crates/factory-db/src/lifecycle.rs": "Session-open recheck and newer-suspend fencing during reconciliation; bounded no-op/failed-resume audit; idempotent same-generation checkpoint/transitions and terminal session audit", "crates/factory-api/src/lifecycle.rs": "Explicit session closure includes pending reconciliation"}, "unchangedCapabilities": "Actual CLI/model invocation, process supervision, scheduler/task claims, worktree/Git execution, profile permission resolution, handoff and conversation execution modules are byte-identical", "presentationDifferences": sorted(file for file in original if file.startswith("apps/dashboard/src/") and ".test." not in file and original[file] != current.get(file)), "nativeCommandCounts": counts, "newResumeHistoryLimit": 1023, "qualification": "Genuine execution evidence predates narrow lifecycle guards. Final controlled/browser and full regression results qualify those changes; this is not a claim that every source file is identical."}
(local / "source-equivalence.json").write_text(json.dumps(value, indent=2) + "\n", encoding="utf8")
print(json.dumps({"verifiedFiles": len(files), "changedLifecycleFiles": sorted(changed), "nativeCommandCounts": counts}))
