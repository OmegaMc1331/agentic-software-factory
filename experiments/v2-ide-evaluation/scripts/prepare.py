import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[3]
EXPERIMENT = ROOT / "experiments/v2-ide-evaluation"
STATE = ROOT / ".factory/v2/p01"


def command(args, cwd):
    subprocess.run(args, cwd=cwd, check=True, capture_output=True, timeout=180)


def fixture(name, count):
    root = STATE / "projects" / name
    if (root / ".asf-p01-ready").exists():
        return root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    (root / ".env").write_text("", encoding="utf-8")
    (root / ".asf-p01-fixture").write_text("ASF_P01_OWNED_FIXTURE_V1", encoding="utf-8")
    (root / ".gitignore").write_text(".factory/\n.env\n", encoding="utf-8")
    (root / "src").mkdir(exist_ok=True)
    (root / "src/main.ts").write_text('export function greet(name: string): string { return `Hello ${name}`; }\nconst wrong: number = "mistake";\nconsole.log(greet("ASF"));\n', encoding="utf-8")
    (root / "src/lib.ts").write_text('export const answer: number = 42;\n', encoding="utf-8")
    (root / "tsconfig.json").write_text('{"compilerOptions":{"strict":true,"noEmit":true,"target":"ES2022"},"include":["src"]}', encoding="utf-8")
    (root / "debug.py").write_text('value = 40\nvalue += 2\nprint(f"RESULT={value}", flush=True)\n', encoding="utf-8")
    (root / "sample.yaml").write_text('project: ASF\ncapabilities:\n  - editor\n  - debugger\n', encoding="utf-8")
    (root / "README.md").write_text('ASF P01 real Git fixture, synthetic source workload.\n', encoding="utf-8")
    for index in range(count):
        parent = root / "tree" / f"group-{index // 100:03}"
        parent.mkdir(parents=True, exist_ok=True)
        (parent / f"module-{index:05}.ts").write_text(f'export const value{index} = {index};\n' * 20, encoding="utf-8")
    (root / "src/large.ts").write_text(''.join(f'export const item{index}: number = {index};\n' for index in range(5000)), encoding="utf-8")
    for args in (["init", "-q", "-b", "main"], ["config", "user.name", "ASF P01 Test"], ["config", "user.email", "p01@example.test"], ["add", "."]):
        command(["git", *args], root)
    if subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=root).returncode:
        command(["git", "commit", "-q", "-m", "P01 representative fixture"], root)
    (root / ".asf-p01-ready").write_text("complete", encoding="utf-8")
    return root.resolve()


def main():
    argparse.ArgumentParser().parse_args()
    STATE.mkdir(parents=True, exist_ok=True)
    projects = {name: str(fixture(name, count)) for name, count in (("small", 10), ("medium", 500), ("large", 10000), ("other", 5))}
    config = {
        "projects": projects,
        "node": shutil.which("node"),
        "python": str((STATE / "python/Scripts/python.exe").resolve()) if os.name == "nt" else str((STATE / "python/bin/python").resolve()),
        "lsp": str((EXPERIMENT / "node_modules/typescript-language-server/lib/cli.mjs").resolve()),
        "pyright": str((EXPERIMENT / "node_modules/pyright/langserver.index.js").resolve()),
        "probe": str((EXPERIMENT / "scripts/terminal_probe.py").resolve()),
        "assets": str((STATE / "dist").resolve()),
    }
    (STATE / "config.json").write_text(json.dumps(config, indent=2), encoding="utf-8")
    sizes = {}
    for name, folder in projects.items():
        files = [p for p in pathlib.Path(folder).rglob("*") if p.is_file() and ".git" not in p.relative_to(folder).parts and ".factory" not in p.relative_to(folder).parts]
        sizes[name] = {"files": len(files), "bytes": sum(p.stat().st_size for p in files)}
    (STATE / "project-sizes.json").write_text(json.dumps(sizes, indent=2), encoding="utf-8")
    print(json.dumps(sizes, indent=2))


if __name__ == "__main__":
    main()
