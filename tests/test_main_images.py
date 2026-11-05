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
        self.assertEqual(spec['concurrency']['cancel-in-progress'], 'false')
        jobs = spec['jobs']
        self.assertEqual(jobs['checks']['if'], "github.ref == 'refs/heads/main'")
        self.assertEqual(jobs['checks']['uses'], './.github/workflows/check.yml')
        self.assertEqual(jobs['checks']['with']['ref'], '${{ github.sha }}')
        self.assertEqual(jobs['build-images']['needs'], 'checks')
        self.assertEqual(jobs['promote-main']['needs'], 'build-images')
        self.assertEqual(jobs['checks']['permissions'], {'contents': 'read'})
        for name in ('build-images', 'promote-main'):
            self.assertEqual(jobs[name]['permissions'], {'contents': 'read', 'packages': 'write'})

    def test_pending_main_is_not_displaced_by_old_retry_or_other_branch(self):
        concurrency = workflow()['concurrency']
        self.assertEqual(concurrency.get('queue', 'single'), 'max')
        self.assertIn('${{ github.ref }}', concurrency['group'])

    def test_builds_both_components_from_the_same_commit_without_release_tags(self):
        job = workflow()['jobs']['build-images']
        self.assertEqual(job['strategy']['matrix']['include'], [
            {'context': 'horizon', 'suffix': 'medius'},
            {'context': 'gameplay', 'suffix': 'game'},
        ])
        checkout = next(s for s in job['steps'] if s.get('uses', '').startswith('actions/checkout@'))
        self.assertEqual(checkout['with']['ref'], '${{ github.sha }}')
        build = next(s for s in job['steps'] if s.get('uses', '').startswith('docker/build-push-action@'))
        self.assertEqual(build['with']['push'], 'true')
        self.assertEqual(build['with']['platforms'], 'linux/amd64')
        self.assertEqual(build['with']['tags'], '${{ steps.source.outputs.image }}:sha-${{ github.sha }}')
        self.assertIn('org.opencontainers.image.revision=${{ github.sha }}', build['with']['labels'])
        self.assertNotIn('main', build['with']['tags'])
        self.assertNotIn('latest', build['with']['tags'])

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
        self.assertEqual(len(containers), 3)
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
                   'TEST_GH_FAIL': '1' if gh_fails else '0'}
            result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', step['run']],
                                    env=env, capture_output=True, text=True, timeout=5)
            calls = [json.loads(line) for line in record.read_text().splitlines()] if record.exists() else []
            return result, calls, summary.read_text() if summary.exists() else ''

    def test_current_commit_promotes_only_gameplay_and_medius_main_aliases(self):
        result, calls, summary = self.run_promotion()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            ['buildx', 'imagetools', 'create', '--prefer-index=false', '--tag',
             'ghcr.io/example/mag-recomp-server-medius:main',
             'ghcr.io/example/mag-recomp-server-medius:sha-' + SHA],
            ['buildx', 'imagetools', 'create', '--prefer-index=false', '--tag',
             'ghcr.io/example/mag-recomp-server-game:main',
             'ghcr.io/example/mag-recomp-server-game:sha-' + SHA],
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
