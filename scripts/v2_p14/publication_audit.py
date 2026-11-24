"""Read-only Git publication screening. Run from the intended publication repository."""

import argparse
from collections import Counter, defaultdict
import hashlib
import html
import json
from pathlib import PurePosixPath, Path
import re
import subprocess
from urllib.parse import unquote

OID = re.compile(r"[0-9a-f]{40}")
PROFILE = re.compile(
    r"(?i)(?:[a-z]:/+(?:users|documents and settings)/|/mnt/[a-z]/users/|"
    r"(?:^|[\s'\"=>(:/])/(?:home|users)/)(?:[^/\s'\"<>,;)|]+)"
)
SESSION = re.compile(
    r"(?i)(?:^|[^\w])\.(?:codex|claude)/(?:sessions|archived_sessions|projects|auth\.json)\b"
)
CREDENTIAL = re.compile(
    r"(?i)(?:\b(?:gh[pousr]_|github_pat_|sk-proj-|sk-ant-|xox[baprs]-)[a-z0-9_-]{20,}"
    r"|-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----)"
)
PRIVATE_DIRECTORIES = {".factory", ".codex", ".claude", ".ssh", ".aws", ".gnupg", "node_modules"}
NON_TEXT = {
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".svg", ".pdf", ".docx", ".zip",
    ".7z", ".gz", ".tar", ".exe", ".msi", ".dll", ".so", ".dylib", ".sqlite", ".sqlite3", ".db",
}


def git(*args):
    return subprocess.check_output(["git", "--no-replace-objects", *args])


def normalized(line):
    for _ in range(2):
        line = unquote(html.unescape(line))
        line = re.sub(r"(?i)\\u00(2f|3a|5c)", lambda match: chr(int(match[1], 16)), line)
    return re.sub(r"\\+/?", "/", line)


def markers(line):
    value = normalized(line)
    if PROFILE.search(value):
        yield "profile-path"
    if SESSION.search(value):
        yield "provider-private-directory"
    if CREDENTIAL.search(value):
        yield "credential-marker"
    if re.search(r"(?i)\bdata:(?:image|application)/(?!json\b)[^,]+;base64,", value):
        yield "embedded-media-review"


def path_reasons(path, mode):
    name = PurePosixPath(path)
    lowered = name.name.lower()
    if mode not in {"100644", "100755"}:
        yield "non-regular-git-path"
    if (
        any(part.lower() in PRIVATE_DIRECTORIES for part in name.parts)
        or path.startswith("docs/v2/evidence/")
        or lowered == ".env"
        or (lowered.startswith(".env.") and lowered != ".env.example")
        or lowered in {"auth.json", "credentials.json", ".npmrc", ".netrc", "id_rsa", "id_ed25519"}
    ):
        yield "private-file-location"
    if name.suffix.lower() in NON_TEXT:
        yield "non-text-artifact-review"
    yield from markers(path)


def object_stream(objects):
    with subprocess.Popen(
        ["git", "--no-replace-objects", "cat-file", "--batch"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    ) as batch:
        for oid in sorted(objects):
            batch.stdin.write(oid.encode() + b"\n")
            batch.stdin.flush()
            actual, kind, size = batch.stdout.readline().decode("ascii").split()
            data = batch.stdout.read(int(size))
            if actual != oid or len(data) != int(size) or batch.stdout.read(1) != b"\n":
                raise ValueError("Object read did not match requested Git blob")
            yield oid, kind, data
        batch.stdin.close()
        if batch.wait():
            raise ValueError("Git object reader failed")


def commit_id(revision):
    return git("rev-parse", "--verify", "--end-of-options", revision + "^{commit}").decode().strip()


def screen(candidate, public_refs, public_emails):
    head = commit_id(candidate)
    refs = sorted(set(public_refs))
    if not refs or any(not OID.fullmatch(oid) for oid in refs):
        raise ValueError("Provide the verified full Git object IDs of the public refs")
    outgoing = set(git("rev-list", "--objects", "--no-object-names", head, "--not", *refs).decode().split())
    commits = git("rev-list", head, "--not", *refs).decode().split()
    tip_paths = Counter()
    path_locations = defaultdict(set)
    findings = []
    for ref in sorted(set(commits) | {head}):
        entries = git("ls-tree", "-r", "-z", ref).rstrip(b"\0")
        for entry in entries.split(b"\0") if entries else ():
            info, name = entry.split(b"\t", 1)
            mode, kind, oid = info.decode("ascii").split()
            path = name.decode("utf-8")
            if ref == head:
                tip_paths[mode] += 1
            location = hashlib.sha256(name).hexdigest()
            for finding in sorted(set(path_reasons(path, mode))):
                findings.append({"object": oid, "kind": finding, "pathSha256": location})
            if kind == "blob":
                path_locations[oid].add(location)

    counts = Counter()
    largest = []
    inspected = []
    for oid, kind, data in object_stream(outgoing | set(path_locations)):
        if oid in outgoing:
            counts[kind] += 1
            inspected.append({
                "id": oid, "kind": kind, "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            })
        if kind in {"commit", "blob", "tag"}:
            try:
                text = data.decode("utf-8")
            except UnicodeDecodeError:
                text = ""
                findings.append({"object": oid, "kind": "non-utf8-artifact-review"})
            if "\0" in text or text.startswith("version https://git-lfs.github.com/spec/v1\n"):
                findings.append({"object": oid, "kind": "binary-or-lfs-review"})
            if kind == "commit":
                if len(emails := re.findall(r"^(?:author|committer) [^\n]+<([^>\n]+)>", text.split("\n\n", 1)[0], re.M)) != 2:
                    findings.append({"object": oid, "kind": "unknown-commit-identity"})
                elif any(email.lower() not in public_emails for email in emails):
                    findings.append({"object": oid, "kind": "unreviewed-commit-email"})
            for line, value in enumerate(text.splitlines(), 1):
                for marker in markers(value):
                    findings.append({"object": oid, "kind": marker, "line": line})
        if kind == "blob":
            largest.append({"id": oid, "bytes": len(data)})

    unique_findings = {json.dumps(row, sort_keys=True): row for row in findings}
    return {
        "candidate": head,
        "publicRefs": refs,
        "outgoingCounts": dict(counts),
        "tipModes": dict(tip_paths),
        "outgoingCommits": commits,
        "outgoingObjects": inspected,
        "screenedBlobs": len(largest),
        "largestBlobs": sorted(largest, key=lambda entry: (-entry["bytes"], entry["id"]))[:10],
        "findings": [unique_findings[key] for key in sorted(unique_findings)],
    }


def review(result, reviews):
    """Permit exact synthetic fixture or manually reviewed media findings by immutable object ID."""
    permitted = {
        "SYNTHETIC_TEST_VALUE": {"profile-path", "provider-private-directory", "credential-marker"},
        "RENDERED_ARTIFACT_REVIEWED": {
            "non-text-artifact-review", "non-utf8-artifact-review",
            "binary-or-lfs-review", "embedded-media-review",
        },
    }
    outstanding = list(result["findings"])
    for entry in reviews:
        finding = entry["finding"]
        if (
            finding not in outstanding
            or finding["kind"] not in permitted.get(entry["classification"], set())
            or not entry.get("reason", "").strip()
        ):
            raise ValueError("Review must match an outstanding, eligible exact finding")
        outstanding.remove(finding)
    return {**result, "reviewedFindings": reviews, "unresolvedFindings": outstanding,
            "result": "BLOCKED" if outstanding else "PASS"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate")
    parser.add_argument("--public-ref", action="append", required=True)
    parser.add_argument("--public-email", action="append", required=True)
    parser.add_argument("--reviews", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = screen(args.candidate, args.public_ref, {email.lower() for email in args.public_email})
    result = review(result, json.loads(args.reviews.read_text(encoding="utf-8")) if args.reviews else [])
    with args.output.open("x", encoding="utf-8", newline="\n") as out:
        json.dump(result, out, indent=2)
        out.write("\n")
    print(json.dumps({key: result[key] for key in ("candidate", "outgoingCounts", "result")}))
    return 1 if result["unresolvedFindings"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
