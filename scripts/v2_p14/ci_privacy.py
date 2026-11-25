"""Publication policy for generated artifacts, independent of source review."""

import hashlib
import json
import os
import re
import stat

from publication_audit import CREDENTIAL, OID, SESSION, normalized


SHA256 = re.compile(r"[a-f0-9]{64}")
PROFILE = re.compile(
    r"(?i)(?:[a-z]:/+(?:users|documents and settings)/|/mnt/[a-z]/users/|"
    r"(?:^|[\s'\x22=>(:/\x00\ufffd])/(?:home|users)/)([^/\s'\x22<>,;)|\x00]+)"
)
HOSTED_PROFILES = frozenset({"runner", "runneradmin"})
AUTH = re.compile(
    r"(?i)(?:\b(?:authorization|proxy-authorization)[\x22']?\s*[:=]\s*[\x22']?\s*(?:bearer|basic)\s+[a-z0-9._+/=-]{12,}"
    r"|\b(?:factory_session|(?:set-)?cookie|x-factory-csrf|api[_-]?key)[\x22']?\s*[:=]\s*[\x22']?\s*[a-z0-9._+/=-]{12,}"
    r"|\b(?:AKIA|ASIA)[A-Z0-9]{16}\b|\beyJ[a-z0-9_-]{8,}\.eyJ[a-z0-9_-]{8,}\.[a-z0-9_-]{16,})"
)
PRIVATE_STORE = re.compile(
    r"(?i)(?:^|/)\.(?:ssh|aws|gnupg)/(?:[^/\x00\s]+)|(?:^|/)\.env(?:[./\x00\s]|$)"
)
ABSOLUTE_PDB = re.compile(r"(?i)[a-z]:/[^\x00\r\n\x22<>]{1,512}\.pdb\b")
WIDE = re.compile(rb"(?:[\x20-\x7e]\x00){5,}|(?:\x00[\x20-\x7e]){5,}")
MAP = re.compile(rb"(?m)(?://[#@]|/\*[#@])\s*sourceMappingURL\s*=")
MEMBER_LIMIT = 150_000_000
TOTAL_LIMIT = 200_000_000
JSON_LIMIT = 4_000_000


class PrivacyError(ValueError):
    """A generic outward failure. Never place a sensitive value in the message."""


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(data):
    return (json.dumps(data, indent=2, ensure_ascii=True, allow_nan=False) + "\n").encode("ascii")


def no_duplicates(pairs):
    result = dict(pairs)
    if len(result) != len(pairs):
        raise PrivacyError("Duplicate JSON field")
    return result


def invalid_constant(_):
    raise PrivacyError("Invalid JSON number")


def parse(data):
    return json.loads(data.decode("utf-8"), object_pairs_hook=no_duplicates, parse_constant=invalid_constant)


def is_link(info):
    return stat.S_ISLNK(info.st_mode) or bool(getattr(info, "st_file_attributes", 0) & 0x400)


def bounded_read(path, limit=MEMBER_LIMIT):
    info = path.lstat()
    if is_link(info) or not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > limit:
        raise PrivacyError("Unexpected file type or size")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
        actual = os.fstat(stream.fileno())
        fields = ("st_dev", "st_ino", "st_size", "st_mode", "st_mtime_ns")
        if any(getattr(actual, key) != getattr(info, key) for key in fields) or len(data) != info.st_size:
            raise PrivacyError("File changed during validation")
    return data


def text_finding(value):
    value = normalized(value)
    if any(rule.search(value) for rule in (SESSION, PRIVATE_STORE, CREDENTIAL, AUTH, ABSOLUTE_PDB)):
        return True
    return any(found[1].lower() not in HOSTED_PROFILES for found in PROFILE.finditer(value))


def screen(data, *, binary=False):
    if not binary and b"\x00" in data:
        raise PrivacyError("Unreviewed binary")
    if text_finding(data.decode("utf-8", errors="replace" if binary else "strict")) or MAP.search(data):
        raise PrivacyError("Sensitive content or source map detected")
    if binary:
        for run in WIDE.finditer(data):
            encoding = "utf-16-le" if data[run.start() + 1] == 0 else "utf-16-be"
            # Include a bounded suffix so a non-ASCII component after an ASCII
            # directory prefix is still checked.
            end = min(len(data), run.end() + 256)
            end -= (end - run.start()) % 2
            if text_finding(data[run.start():end].decode(encoding, errors="replace")):
                raise PrivacyError("Sensitive UTF-16 content detected")
