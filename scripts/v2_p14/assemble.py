import argparse
from pathlib import Path
import re
import shutil
import tempfile

from package import TARGETS, sha, unpack, verify_machine


def assemble(directory, output, source):
    if not re.fullmatch("[a-f0-9]{40}", source) or output.exists():
        raise ValueError("Exact source and new output directory required")
    directory = directory.resolve()
    verified = []
    for target, (_, _, name, executable) in TARGETS.items():
        matches = list(directory.rglob(name))
        if len(matches) != 1 or not matches[0].resolve().is_relative_to(directory):
            raise ValueError("Missing, duplicate or escaped native artifact: " + name)
        archive = matches[0]
        checksum = archive.with_name(name + ".sha256")
        if not checksum.resolve().is_relative_to(directory) or checksum.read_text(encoding="ascii").strip() != f"{sha(archive)}  {name}":
            raise ValueError("Artifact checksum mismatch")
        with tempfile.TemporaryDirectory(prefix="asf-p14-integrity-") as temporary:
            extracted = Path(temporary) / "unpacked"
            manifest = unpack(archive, extracted)
            if manifest["sourceCommit"] != source or manifest["target"] != target:
                raise ValueError("Native artifact/source identity mismatch")
            verify_machine((extracted / executable).read_bytes(), target)
        verified.extend([archive, checksum])
    output.mkdir(parents=True)
    for file in verified:
        shutil.copyfile(file, output / file.name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source", required=True)
    args = parser.parse_args()
    assemble(args.input, args.output, args.source)
