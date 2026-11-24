import json
import pathlib
import sqlite3
import tempfile

fixture = pathlib.Path(tempfile.mkdtemp(prefix="asf-p07-wal-")) / "fixture.sqlite3"
reader = sqlite3.connect(fixture, timeout=0.2, isolation_level=None)
writer = sqlite3.connect(fixture, timeout=0.2, isolation_level=None)
reader.execute("pragma journal_mode=WAL")
reader.execute("create table observations(id primary key,value)")
reader.execute("insert into observations values(1,'loaded')")
reader.execute("begin")
reader.execute("select value from observations").fetchone()
writer.execute("update observations set value='external'")
try:
    reader.execute("update observations set value='stale'")
    failure = {"unexpected": "success"}
except sqlite3.OperationalError as error:
    failure = {
        "error": str(error),
        "sqlite_errorcode": error.sqlite_errorcode,
        "sqlite_errorname": error.sqlite_errorname,
    }
reader.execute("rollback")
reader.execute("begin immediate")
observed = reader.execute("select value from observations").fetchone()[0]
reader.execute("update observations set value='revalidated'")
reader.execute("commit")
report = {
    "sqlite_version": sqlite3.sqlite_version,
    "deferred_snapshot_upgrade": failure,
    "immediate_transaction_observed": observed,
    "final": reader.execute("select value from observations").fetchone()[0],
    "scope": "Deterministic SQLite WAL reproducer; production symptom inferred from deferred transaction code and completed-run/failed-summary evidence.",
}
assert failure.get("sqlite_errorname") == "SQLITE_BUSY_SNAPSHOT", report
assert observed == "external" and report["final"] == "revalidated", report
output = pathlib.Path(__file__).resolve().parents[2] / ".factory/v2/p07/attempts/wal-snapshot-reproduction.json"
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(report, indent=2), encoding="utf8")
reader.close()
writer.close()
print(json.dumps(report))
