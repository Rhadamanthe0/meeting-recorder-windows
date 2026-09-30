"""Regression de sync-upstream : depots temporaires locaux, GitHub simule."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

WORKFLOW = Path(__file__).resolve().parents[1] / 'workflows' / 'sync-upstream.yml'
BASH = shutil.which('bash')
if os.name == 'nt':
    BASH = str(Path(shutil.which('git')).resolve().parents[1] / 'usr/bin/bash.exe')


def git(repo, *args):
    return subprocess.check_output(['git', '-c', 'core.autocrlf=false', '-C', str(repo), *args], text=True, stderr=subprocess.PIPE).strip()


def write(repo, name, text):
    (repo / name).write_text(text, encoding='utf-8', newline='\n')


def commit(repo, message):
    git(repo, 'add', '.')
    git(repo, 'commit', '-m', message)
    return git(repo, 'rev-parse', 'HEAD')


def workflow_script():
    source = WORKFLOW.read_text(encoding='utf-8')
    step = source.split('      - name: Merge upstream, bump patch, open PR, and run CI\n', 1)[1]
    body = step.split('        run: |\n', 1)[1].split('          # Dispatch explicite', 1)[0]
    return '\n'.join(line[10:] for line in body.splitlines()) + '\n'


class SyncUpstreamTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sync-upstream-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.origin = self.root / 'origin.git'
        self.seed = self.root / 'seed'
        self.runner = self.root / 'runner'
        self.env = os.environ.copy()
        self.env.update(GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
                        GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid',
                        GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                        GITHUB_RUN_ID='999', GIT_CONFIG_COUNT='1',
                        GIT_CONFIG_KEY_0='core.autocrlf', GIT_CONFIG_VALUE_0='false',
                        GIT_ALLOW_PROTOCOL='file')
        subprocess.run(['git', 'init', '--bare', str(self.origin)], check=True, capture_output=True)
        subprocess.run(['git', 'init', '-b', 'master', str(self.seed)], check=True, capture_output=True)
        git(self.seed, 'config', 'user.name', 'Test')
        git(self.seed, 'config', 'user.email', 'test@example.invalid')
        write(self.seed, 'Cargo.toml', '[package]\nname = "meeting-recorder-windows"\nversion = "0.1.1"\n')
        write(self.seed, 'Cargo.lock', '[[package]]\nname = "meeting-recorder-windows"\nversion = "0.1.1"\n')
        write(self.seed, 'shared.txt', 'base\n')
        self.base = commit(self.seed, 'base')
        git(self.seed, 'branch', 'upstream')
        write(self.seed, 'windows.txt', 'port\n')
        commit(self.seed, 'Windows port')
        git(self.seed, 'remote', 'add', 'origin', str(self.origin))
        git(self.seed, 'push', 'origin', 'master')
        git(self.seed, 'checkout', 'upstream')
        write(self.seed, 'linux.txt', 'upstream one\n')
        self.upstream = commit(self.seed, 'upstream one')
        git(self.seed, 'checkout', '-b', 'sync/upstream-42', 'master')
        git(self.seed, 'merge', 'upstream', '--no-edit')
        self.bump()
        self.original = git(self.seed, 'rev-parse', 'HEAD')
        git(self.seed, 'push', 'origin', 'sync/upstream-42')
        self.tools = self.root / 'tools'
        self.tools.mkdir()
        write(self.tools, 'gh', '#!/bin/bash\nif [ "$1 $2" = "pr list" ]; then\n echo \'{"number":42,"headRefName":"sync/upstream-42"}\'\nelse\n printf "%s\\n" "$*" >> "$TEST_EVENTS"\nfi\n')
        write(self.tools, 'jq', '#!/bin/bash\ncat >/dev/null\ncase "$2" in\n .number) echo 42 ;;\n .headRefName) echo sync/upstream-42 ;;\n *) exit 1 ;;\nesac\n')
        # Sous POSIX, Bash ignore les scripts non executables dans PATH.
        for name in ('gh', 'jq'):
            (self.tools / name).chmod(0o755)
        self.events = self.root / 'events'
        self.env.update(TEST_EVENTS=str(self.events), TEST_TOOLS=str(self.tools),
                        PATH=str(self.tools) + os.pathsep + str(Path(BASH).parent) + os.pathsep + self.env['PATH'])

    def bump(self):
        for name in ('Cargo.toml', 'Cargo.lock'):
            write(self.seed, name, (self.seed / name).read_text().replace('0.1.1', '0.1.2'))
        commit(self.seed, 'bump')

    def run_workflow(self):
        subprocess.run(['git', '-c', 'core.autocrlf=false', 'clone', '--single-branch', '--branch', 'master',
                        str(self.origin), str(self.runner)], check=True, capture_output=True)
        git(self.runner, 'remote', 'add', 'upstream', str(self.seed))
        git(self.runner, 'fetch', 'upstream', 'upstream:refs/remotes/upstream/main')
        script = self.root / 'workflow.sh'
        # Refuser tout CLI reel si la resolution des simulations regresse.
        guard = '''for tool in gh jq; do
  [[ -x "$TEST_TOOLS/$tool" && "$(type -P "$tool")" -ef "$TEST_TOOLS/$tool" ]] || exit 99
done
'''
        script.write_text(guard + workflow_script(), encoding='utf-8', newline='\n')
        # Les remotes sont des chemins locaux. Aucun acces reseau ni appel GitHub.
        result = subprocess.run([BASH, str(script)], cwd=self.runner, env=self.env,
                                text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn('pr create', self.events.read_text() if self.events.exists() else '')
        return result.stdout

    def test_mock_cli_precedes_executable_fallback(self):
        fallback = self.root / 'fallback'
        fallback.mkdir()
        for name in ('gh', 'jq'):
            write(fallback, name, '#!/bin/bash\necho SAFE_FALLBACK\n')
            (fallback / name).chmod(0o755)
        env = self.env.copy()
        env['PATH'] = str(self.tools) + os.pathsep + str(fallback) + os.pathsep + env['PATH']
        # Une sentinelle locale capture tout repli, sans appeler de CLI reel.
        result = subprocess.run([BASH, '-c', 'gh pr list; printf "{}" | jq -r .number'],
                                env=env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout.splitlines(),
                         ['{"number":42,"headRefName":"sync/upstream-42"}', '42'])
        self.assertFalse(self.events.exists())

    def test_identical_merge_and_bump_in_fresh_checkout(self):
        output = self.run_workflow()
        self.assertIn('covers this delta', output)
        self.assertFalse(self.events.exists())
        self.assertEqual(git(self.origin, 'rev-parse', 'sync/upstream-42'), self.original)

    def test_identical_fast_forward_and_bump(self):
        git(self.seed, 'checkout', '-B', 'sync/upstream-42', 'upstream')
        self.bump()
        self.original = git(self.seed, 'rev-parse', 'HEAD')
        git(self.seed, 'push', '--force', 'origin', 'sync/upstream-42')
        git(self.seed, 'push', '--force', 'origin', self.base + ':master')
        self.assertIn('covers this delta', self.run_workflow())
        self.assertFalse(self.events.exists())
        self.assertEqual(git(self.origin, 'rev-parse', 'sync/upstream-42'), self.original)

    def test_advanced_upstream_reuses_branch_and_version(self):
        git(self.seed, 'checkout', 'upstream')
        write(self.seed, 'linux.txt', 'upstream two\n')
        self.upstream = commit(self.seed, 'upstream two')
        self.run_workflow()
        tip = git(self.origin, 'rev-parse', 'sync/upstream-42')
        self.assertNotEqual(tip, self.original)
        git(self.origin, 'merge-base', '--is-ancestor', self.upstream, tip)
        self.assertIn('version = "0.1.2"', git(self.origin, 'show', tip + ':Cargo.toml'))
        self.assertIn('pr comment 42', self.events.read_text())

    def test_advanced_master_reuses_branch(self):
        git(self.seed, 'checkout', 'master')
        write(self.seed, 'windows.txt', 'port two\n')
        master = commit(self.seed, 'port two')
        git(self.seed, 'push', 'origin', 'master')
        self.run_workflow()
        tip = git(self.origin, 'rev-parse', 'sync/upstream-42')
        git(self.origin, 'merge-base', '--is-ancestor', master, tip)
        self.assertNotEqual(tip, self.original)

    def test_conflict_keeps_existing_pr_branch(self):
        git(self.seed, 'checkout', 'master')
        write(self.seed, 'shared.txt', 'Windows change\n')
        commit(self.seed, 'Windows change')
        git(self.seed, 'push', 'origin', 'master')
        git(self.seed, 'checkout', 'upstream')
        write(self.seed, 'shared.txt', 'Linux change\n')
        commit(self.seed, 'Linux change')
        self.assertIn('branche existante inchangee', self.run_workflow())
        self.assertEqual(git(self.origin, 'rev-parse', 'sync/upstream-42'), self.original)
        self.assertIn('mise à jour manuelle requise', self.events.read_text(encoding='utf-8'))

    def test_runs_are_serialized(self):
        self.assertIn('concurrency:\n  group: sync-upstream\n  cancel-in-progress: false',
                      WORKFLOW.read_text(encoding='utf-8'))


if __name__ == '__main__':
    unittest.main(verbosity=2)
