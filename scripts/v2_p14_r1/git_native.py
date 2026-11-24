import argparse
from collections import Counter
from datetime import UTC, datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
from time import perf_counter


SIZE = 6000
PATHS = tuple(f'module_{i}.py' for i in range(SIZE))


def git(repo, *args, trace=None):
    env = {key: value for key, value in os.environ.items() if not key.upper().startswith(('GIT_', 'SSH_'))}
    env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
               GIT_LITERAL_PATHSPECS='1', GIT_OPTIONAL_LOCKS='0', GIT_TERMINAL_PROMPT='0', LC_ALL='C')
    if trace:
        env['GIT_TRACE2_EVENT'] = str(trace)
    return subprocess.run(['git', '-c', 'core.fsmonitor=false', '-c', 'core.untrackedCache=false',
                           '-c', 'submodule.recurse=false', *args], cwd=repo, env=env,
                          check=True, capture_output=True, timeout=240).stdout


def content(index, offset):
    return f'VALUE = {index + offset}\n'.encode()


def blob_id(data, object_format):
    return hashlib.new(object_format, f'blob {len(data)}\0'.encode() + data).hexdigest().encode()


def index_entries(raw):
    entries = {}
    for row in raw.strip(b'\0').split(b'\0'):
        info, path = row.split(b'\t', 1)
        entries[path.decode()] = info.split()[1]
    return entries


def run(seed, repo, selection, trace, packed):
    shutil.copytree(seed, repo)
    expected = {}
    object_format = git(repo, 'rev-parse', '--show-object-format').decode().strip()
    for number, path in enumerate(PATHS):
        data = content(number, 200000)
        (repo / path).write_bytes(data)
        expected[path] = blob_id(data, object_format)
    (repo / 'unrelated.py').write_bytes(b'UNCHANGED = False\n')
    (repo / 'untracked.py').write_bytes(b'UNTRACKED = True\n')
    tree = git(repo, 'rev-parse', 'HEAD').strip()
    before = len(list((repo / '.git/objects/pack').glob('*.pack')))
    git(repo, 'status', '--porcelain=v2', '-z', '--untracked-files=all')
    options = ('-c', 'core.bigFileThreshold=0') if packed else ()
    start = perf_counter()
    git(repo, *options, 'add', '--all', f'--pathspec-from-file={selection}', '--pathspec-file-nul', trace=trace)
    stage_ms = (perf_counter() - start) * 1000
    actual = index_entries(git(repo, 'ls-files', '--stage', '-z'))
    assert all(actual[name] == digest for name, digest in expected.items())
    assert set(git(repo, 'diff', '--cached', '--name-only', '-z').strip(b'\0').decode().split('\0')) == set(PATHS)
    start = perf_counter()
    git(repo, 'commit', '-qm', 'Verify exactly the selected paths')
    commit_ms = (perf_counter() - start) * 1000
    assert git(repo, 'rev-parse', 'HEAD^').strip() == tree
    entries = {}
    for row in git(repo, 'ls-tree', '-r', '-z', 'HEAD').strip(b'\0').split(b'\0'):
        info, name = row.split(b'\t', 1)
        entries[name.decode()] = info.split()[2]
    assert entries == actual
    assert git(repo, 'diff', '--name-only', '-z') == b'unrelated.py\0'
    assert (repo / 'untracked.py').read_bytes() == b'UNTRACKED = True\n'
    assert git(repo, 'status', '--porcelain=v1', '-z').endswith(b'?? untracked.py\0')
    assert git(repo, 'config', '--local', '--list').lower().find(b'bigfilethreshold') == -1
    after = len(list((repo / '.git/objects/pack').glob('*.pack')))
    events = [json.loads(line) for line in trace.read_text(encoding='utf-8').splitlines()]
    regions = Counter(event.get('category') for event in events if event['event'] == 'region_leave')
    return {'packedAdd': packed, 'stageMs': stage_ms, 'commitMs': commit_ms,
            'verifiedSelectedIndexEntries': len(expected), 'commitTreeMatchedIndex': True,
            'unrelatedEditAndUntrackedFilePreserved': True, 'newPacks': after - before,
            'traceSha256': hashlib.sha256(trace.read_bytes()).hexdigest(), 'traceRegions': sorted(regions.items())}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('output', type=Path)
    parser.add_argument('--trials', type=int, choices=range(1, 6), default=3)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    temp_root = Path(tempfile.gettempdir()).resolve(strict=True)
    assert shutil.disk_usage(temp_root).free >= 200 * 1024 ** 2
    record = {'testId': 'GIT-R1-NATIVE-COLD-OBJECTS', 'sourceCommit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'status': 'RUNNING', 'startedAt': datetime.now(UTC).isoformat(), 'os': platform.platform(),
              'cpu': platform.processor(), 'logicalCpus': os.cpu_count(), 'gitVersion': git(output, '--version').decode().strip(),
              'fixture': {'selected': SIZE, 'tracked': SIZE + 2, 'unrelatedModified': 1, 'untracked': 1,
                          'gitConfig': {'core.autocrlf': False, 'commit.gpgsign': False, 'gc.auto': 0},
                          'objectState': 'Same initial committed object store cloned by copying; each working-tree replacement is a new blob.'},
              'scope': 'Native Git, no ASF API. The disposable seed uses packed storage in both arms. Only the selected add command differs. A real status scan warms the worktree before both add arms.',
              'samples': []}
    report = output / 'results.json'

    def save():
        report.write_text(json.dumps(record, indent=2) + '\n', encoding='utf-8')

    save()
    try:
        with tempfile.TemporaryDirectory(prefix='asf-p14-r1-native-git-', dir=temp_root) as tmp:
            root = Path(tmp).resolve(strict=True)
            assert root.parent == temp_root and root.name.startswith('asf-p14-r1-native-git-')
            seed = root / 'seed'
            seed.mkdir()
            git(seed, 'init', '-q', '-b', 'main')
            for key, value in [('user.name', 'ASF qualification'), ('user.email', 'qualification@example.invalid'),
                               ('core.autocrlf', 'false'), ('commit.gpgsign', 'false'), ('gc.auto', '0')]:
                git(seed, 'config', key, value)
            (seed / '.gitignore').write_bytes(b'.factory/\n')
            (seed / 'unrelated.py').write_bytes(b'UNCHANGED = True\n')
            for number, path in enumerate(PATHS):
                (seed / path).write_bytes(content(number, 0))
            git(seed, '-c', 'core.bigFileThreshold=0', 'add', '--', '.')
            git(seed, 'commit', '-qm', 'Disposable qualification seed')
            selection = root / 'paths.nul'
            selection.write_bytes(b'\0'.join(path.encode() for path in PATHS) + b'\0')
            for trial in range(1, args.trials + 1):
                for packed in ((False, True) if trial % 2 else (True, False)):
                    label = f'{trial}-{"packed" if packed else "native"}'
                    with tempfile.TemporaryDirectory(prefix=label + '-', dir=root) as isolated:
                        sample = run(seed, Path(isolated) / 'repository', selection, output / f'{label}.trace.jsonl', packed)
                    record['samples'].append({'trial': trial, **sample})
                    save()
        record['summary'] = {}
        for label, packed in [('native', False), ('packed', True)]:
            rows = [row for row in record['samples'] if row['packedAdd'] == packed]
            record['summary'][label] = {name: {'min': min(values), 'median': statistics.median(values), 'max': max(values)}
                                        for name in ('stageMs', 'commitMs')
                                        if (values := [row[name] for row in rows])}
        record['status'] = 'PASS'
    except BaseException as error:
        record.update(status='FAIL', error=repr(error))
        raise
    finally:
        record['finishedAt'] = datetime.now(UTC).isoformat()
        save()
    print(json.dumps(record['summary']))


if __name__ == '__main__':
    main()
