"""Exercise the main image publication graph and its registry command boundary offline."""
import json
import os
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / '.github/workflows/main-images.yml'
SHA = 'a' * 40


def workflow():
    if not WORKFLOW.exists():
        raise AssertionError('the rolling main image workflow is missing')
    return yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)


class MainImagesTests(unittest.TestCase):
    def test_source_checks_run_once_per_event(self):
        spec = yaml.load((ROOT / '.github/workflows/check.yml').read_text(), Loader=yaml.BaseLoader)
        # main pushes reach check.yml through main-images.yml; its own push
        # trigger would duplicate every PR-branch and main run.
        self.assertEqual(set(spec['on']), {'pull_request', 'workflow_call'})

    def test_only_checked_main_builds_can_publish(self):
        spec = workflow()
        self.assertEqual(set(spec['on']), {'push', 'workflow_dispatch'})
        self.assertEqual(spec['on']['push']['branches'], ['main'])
        self.assertEqual(spec['permissions'], {})
        jobs = spec['jobs']
        self.assertEqual(jobs['plan']['if'], "github.ref == 'refs/heads/main'")
        self.assertEqual(jobs['checks']['needs'], 'plan')
        self.assertEqual(jobs['checks']['uses'], './.github/workflows/check.yml')
        self.assertEqual(jobs['checks']['with']['ref'], '${{ github.sha }}')
        self.assertEqual(jobs['checks']['with']['checks'], '${{ needs.plan.outputs.checks }}')
        self.assertEqual(jobs['build-images']['needs'], ['plan', 'checks'])
        self.assertEqual(jobs['promote-main']['needs'], ['plan', 'checks', 'build-images'])
        promote = jobs['promote-main']['if']
        self.assertIn('!cancelled()', promote)
        self.assertIn("needs.checks.result == 'success'", promote)
        self.assertNotIn('always()', promote)
        self.assertEqual(jobs['plan']['permissions'], {'contents': 'read', 'packages': 'read'})
        self.assertEqual(jobs['checks']['permissions'], {'contents': 'read'})
        for name in ('build-images', 'promote-main'):
            self.assertEqual(jobs[name]['permissions'], {'contents': 'read', 'packages': 'write'})

    def test_newest_main_cancels_older_runs_but_reruns_cannot_displace_it(self):
        concurrency = workflow()['concurrency']
        self.assertEqual(concurrency['cancel-in-progress'], 'true')
        self.assertIn('${{ github.ref }}', concurrency['group'])
        # A rerun of an old commit gets its own group instead of cancelling current main.
        self.assertIn("github.run_attempt != '1' && '-rerun'", concurrency['group'])

    def test_builds_only_changed_components_from_the_same_commit_without_release_tags(self):
        job = workflow()['jobs']['build-images']
        self.assertEqual(job['if'], "needs.plan.outputs.build == 'true'")
        self.assertEqual(job['strategy']['matrix'], '${{ fromJSON(needs.plan.outputs.matrix) }}')
        checkout = next(s for s in job['steps'] if s.get('uses', '').startswith('actions/checkout@'))
        self.assertEqual(checkout['with']['ref'], '${{ github.sha }}')
        build = next(s for s in job['steps'] if s.get('uses', '').startswith('docker/build-push-action@'))
        self.assertEqual(build['with']['push'], 'true')
        self.assertEqual(build['with']['platforms'], 'linux/amd64')
        self.assertEqual(build['with']['tags'].splitlines(), [
            '${{ steps.source.outputs.image }}:sha-${{ github.sha }}',
            '${{ steps.source.outputs.image }}:tree-${{ matrix.tree }}'])
        self.assertIn('org.opencontainers.image.revision=${{ github.sha }}', build['with']['labels'])
        self.assertNotIn(':main', build['with']['tags'])
        self.assertNotIn('latest', build['with']['tags'])

    def run_plan(self, existing=(), diff='', event='push', before='b' * 40, fetch_fails=False, rebuild='false'):
        step = next(s for s in workflow()['jobs']['plan']['steps'] if s.get('id') == 'plan')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / 'output'
            summary = root / 'summary.md'
            git = root / 'git'
            git.write_text('#!/usr/bin/env python3\nimport os, sys\n'
                           'a = sys.argv[1:]\n'
                           'if a[0] == "fetch": sys.exit(1 if os.environ["TEST_FETCH_FAIL"] == "1" else 0)\n'
                           'if a[0] == "diff": print(os.environ["TEST_DIFF"], end=""); sys.exit(0)\n'
                           'if a[0] == "rev-parse": print(a[1].split(":")[1] + "-tree"); sys.exit(0)\n'
                           'sys.exit(3)\n')
            docker = root / 'docker'
            docker.write_text('#!/usr/bin/env python3\nimport os, sys\n'
                              'ref = sys.argv[-1]\n'
                              'sys.exit(0 if ref in os.environ["TEST_EXISTING"].split() else 1)\n')
            git.chmod(0o700)
            docker.chmod(0o700)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'GITHUB_EVENT_NAME': event, 'GITHUB_REPOSITORY_OWNER': 'Example',
                   'GITHUB_OUTPUT': str(output), 'GITHUB_STEP_SUMMARY': str(summary),
                   'BEFORE': before, 'REBUILD': rebuild, 'TEST_DIFF': diff,
                   'TEST_FETCH_FAIL': '1' if fetch_fails else '0',
                   'TEST_EXISTING': ' '.join('ghcr.io/example/mag-recomp-server-' + e for e in existing)}
            result = subprocess.run(['bash', '-e', '-c', step['run']], cwd=ROOT,
                                    env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            values = dict(line.split('=', 1) for line in output.read_text().splitlines())
            return values, set(values['checks'].split()), json.loads(values['matrix'])['include']

    ALL_BUILT = ('medius:tree-horizon-tree', 'game:tree-gameplay-tree', 'portal:tree-portal-tree')

    def test_portal_only_merge_builds_and_checks_only_the_portal(self):
        values, checks, include = self.run_plan(existing=self.ALL_BUILT[:2], diff='portal/src/main.rs\n')
        self.assertEqual(include, [{'context': 'portal', 'suffix': 'portal', 'tree': 'portal-tree'}])
        self.assertEqual(checks, {'boundary', 'portal'})
        self.assertEqual(values['build'], 'true')
        self.assertEqual((values['medius_tree'], values['game_tree'], values['portal_tree']),
                         ('horizon-tree', 'gameplay-tree', 'portal-tree'))

    def test_docs_only_merge_builds_nothing_and_runs_the_boundary(self):
        values, checks, include = self.run_plan(existing=self.ALL_BUILT, diff='docs/releasing.md\n')
        self.assertEqual((values['build'], include, checks), ('false', [], {'boundary'}))

    def test_cancelled_commit_components_are_still_built_and_checked(self):
        # The previous commit changed gameplay but its run was cancelled: its tree has no image.
        _, checks, include = self.run_plan(existing=self.ALL_BUILT[::2], diff='portal/README.md\n')
        self.assertEqual([i['suffix'] for i in include], ['game'])
        self.assertEqual(checks, {'boundary', 'gameplay', 'registry', 'portal'})

    def test_dispatch_unknown_diff_or_rebuild_checks_or_builds_everything(self):
        for kwargs in ({'event': 'workflow_dispatch'}, {'fetch_fails': True}, {'before': ''}):
            _, checks, include = self.run_plan(existing=self.ALL_BUILT, **kwargs)
            self.assertIn('all', checks, kwargs)
            self.assertEqual(include, [], kwargs)
        _, _, include = self.run_plan(existing=self.ALL_BUILT, diff='docs/a.md\n', rebuild='true')
        self.assertEqual([i['suffix'] for i in include], ['medius', 'game', 'portal'])

    def test_main_test_values_render_fresh_images_and_clear_existing_digest_pins(self):
        with tempfile.NamedTemporaryFile(mode='w', suffix='.yaml') as pinned:
            pinned.write('images:\n  medius:\n    digest: sha256:' + 'b' * 64 + '\n  gameplay:\n    digest: sha256:' + 'c' * 64 + '\n')
            pinned.flush()
            result = subprocess.run([shutil.which('helm') or '/opt/homebrew/bin/helm',
                                     'template', 'prerelease', str(ROOT / 'charts/mag-recomp-server'),
                                     '-f', str(ROOT / 'charts/mag-recomp-server/ci-values.yaml'),
                                     '-f', pinned.name, '-f', str(ROOT / 'docs/main-images-values.yaml')],
                                    capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        containers = [c for d in yaml.safe_load_all(result.stdout) if d['kind'] == 'Deployment'
                      for c in d['spec']['template']['spec']['containers']]
        # Medius and MAPS, plus one gameplay container per configured worker (or the single one).
        workers = yaml.safe_load((ROOT / 'charts/mag-recomp-server/ci-values.yaml').read_text()).get('gameplayWorkers') or [None]
        self.assertEqual(len(containers), 2 + len(workers))
        for container in containers:
            self.assertEqual(container['imagePullPolicy'], 'Always')
            suffix = 'game' if container['name'] == 'gameplay' else 'medius'
            self.assertEqual(container['image'], 'ghcr.io/tekulvw/mag-recomp-server-' + suffix + ':main')

    def run_promotion(self, current_sha=SHA, gh_fails=False):
        step = next(s for s in workflow()['jobs']['promote-main']['steps'] if s.get('name') == 'Promote current main images')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = root / 'commands.jsonl'
            summary = root / 'summary.md'
            gh = root / 'gh'
            gh.write_text('#!/usr/bin/env python3\nimport os, sys\n'
                          'if os.environ.get("TEST_GH_FAIL") == "1": sys.exit(1)\n'
                          'print(os.environ["TEST_MAIN_SHA"])\n')
            docker = root / 'docker'
            docker.write_text('#!/usr/bin/env python3\nimport json, os, sys\n'
                              'with open(os.environ["TEST_COMMAND_LOG"], "a") as f: f.write(json.dumps(sys.argv[1:]) + "\\n")\n')
            gh.chmod(0o700)
            docker.chmod(0o700)
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'GITHUB_SHA': SHA, 'GITHUB_REPOSITORY': 'Example/mag-recomp-server',
                   'GITHUB_REPOSITORY_OWNER': 'Example', 'GITHUB_STEP_SUMMARY': str(summary),
                   'TEST_COMMAND_LOG': str(record), 'TEST_MAIN_SHA': current_sha,
                   'MEDIUS_TREE': 'm' * 40, 'GAME_TREE': 'g' * 40, 'PORTAL_TREE': 'p' * 40,
                   'TEST_GH_FAIL': '1' if gh_fails else '0'}
            result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', step['run']],
                                    env=env, capture_output=True, text=True, timeout=5)
            calls = [json.loads(line) for line in record.read_text().splitlines()] if record.exists() else []
            return result, calls, summary.read_text() if summary.exists() else ''

    def test_current_commit_promotes_each_component_main_alias(self):
        result, calls, summary = self.run_promotion()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ['buildx', 'imagetools', 'create', '--prefer-index=false',
             '--tag', 'ghcr.io/example/mag-recomp-server-' + suffix + ':sha-' + SHA,
             '--tag', 'ghcr.io/example/mag-recomp-server-' + suffix + ':main',
             'ghcr.io/example/mag-recomp-server-' + suffix + ':tree-' + tree * 40]
            for suffix, tree in (('medius', 'm'), ('game', 'g'), ('portal', 'p'))
        ])
        self.assertIn(SHA, summary)

    def test_old_rerun_cannot_move_main_backwards(self):
        result, calls, summary = self.run_promotion('b' * 40)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [])
        self.assertIn('Skipping', summary)

    def test_unknown_current_main_fails_before_any_promotion(self):
        result, calls, _ = self.run_promotion(gh_fails=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, [])
