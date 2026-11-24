"""Exercise which source checks pull requests and callers select, offline and with invented paths."""
from pathlib import Path
import subprocess
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SELECT = ROOT / '.github/scripts/select-checks.sh'
CHECKS = ('boundary', 'python', 'gameplay', 'horizon', 'registry', 'portal', 'chart', 'release')


def select(*args, paths=None):
    result = subprocess.run(['bash', str(SELECT), *args], input='\n'.join(paths or []) + '\n',
                            capture_output=True, text=True, timeout=10)
    if result.returncode:
        raise AssertionError(result.stderr)
    values = dict(line.split('=', 1) for line in result.stdout.splitlines())
    if tuple(values) != CHECKS:
        raise AssertionError(result.stdout)
    return {name for name, value in values.items() if value == 'true'}


def changed(*paths):
    return select('changed', paths=list(paths))


class PullRequestSelectionTests(unittest.TestCase):
    def test_component_changes_run_only_their_checks(self):
        self.assertEqual(changed('portal/src/main.rs', 'portal/templates/index.html'), {'boundary', 'portal'})
        self.assertEqual(changed('gameplay/src/replication.rs'), {'boundary', 'gameplay', 'registry'})
        self.assertEqual(changed('horizon/Server.Medius/Program.cs'), {'boundary', 'horizon', 'registry'})
        self.assertEqual(changed('tests/registry_fixture.rs'), {'boundary', 'python', 'registry'})

    def test_docs_only_changes_run_the_publication_boundary(self):
        self.assertEqual(changed('docs/releasing.md', 'README.md', 'AGENTS.md'), {'boundary'})

    def test_tools_tests_and_chart_inputs(self):
        self.assertEqual(changed('tools/navmesh/prepare.py'), {'boundary', 'python'})
        self.assertEqual(changed('charts/mag-recomp-server/values.yaml'), {'boundary', 'python', 'chart'})
        self.assertEqual(changed('tools/data-bundle.py'), {'boundary', 'python', 'chart'})
        self.assertEqual(changed('docs/main-images-values.yaml'), {'boundary', 'python'})
        # The Medius copy of the portal ticket vectors is compared by a Python test.
        self.assertEqual(changed('portal/tests/ticket-vectors.json'), {'boundary', 'python', 'portal'})

    def test_release_pull_request_checks_version_files_and_routing(self):
        self.assertEqual(changed('.release-please-manifest.json', 'CHANGELOG.md', 'portal/CHANGELOG.md',
                                 'portal/Cargo.toml', 'portal/Cargo.lock'), {'boundary', 'portal', 'release'})
        self.assertEqual(changed('charts/mag-recomp-server/Chart.yaml'), {'boundary', 'python', 'chart', 'release'})

    def test_workflow_selector_or_unknown_diff_runs_everything(self):
        self.assertEqual(changed('.github/workflows/check.yml'), set(CHECKS))
        self.assertEqual(changed('.github/scripts/select-checks.sh'), set(CHECKS))
        self.assertEqual(changed(), set(CHECKS))


class CallerSelectionTests(unittest.TestCase):
    def test_named_checks(self):
        self.assertEqual(select('list', 'all'), set(CHECKS))
        self.assertEqual(select('list', 'chart', 'release'), {'chart', 'release'})
        self.assertEqual(select('list', 'boundary', 'portal', 'all'), set(CHECKS))

    def test_unknown_check_fails_visibly(self):
        with self.assertRaises(AssertionError):
            select('list', 'gamplay')


class WorkflowGatingTests(unittest.TestCase):
    def spec(self, name):
        return yaml.load((ROOT / '.github/workflows' / name).read_text(), Loader=yaml.BaseLoader)

    def test_pull_request_runs_are_cancelled_by_new_pushes_and_closing(self):
        for name in ('check.yml', 'pr-title.yml'):
            spec = self.spec(name)
            self.assertIn('closed', spec['on']['pull_request']['types'], name)
            self.assertIn('github.event.pull_request.number', spec['concurrency']['group'], name)
        self.assertEqual(self.spec('pr-title.yml')['concurrency']['cancel-in-progress'], 'true')
        # Callers (main images, release tags) are never cancelled by this workflow's group.
        self.assertEqual(self.spec('check.yml')['concurrency']['cancel-in-progress'],
                         "${{ github.event_name == 'pull_request' }}")
        self.assertEqual(self.spec('pr-title.yml')['jobs']['pr-title']['if'], "github.event.action != 'closed'")

    def test_every_check_job_is_gated_by_the_selection(self):
        jobs = self.spec('check.yml')['jobs']
        self.assertEqual(jobs['changes']['if'], "github.event.action != 'closed'")
        self.assertEqual(set(jobs['changes']['outputs']), set(CHECKS))
        for name, job in jobs.items():
            if name != 'changes':
                self.assertEqual(job['needs'], 'changes', name)
                self.assertIn('needs.changes.outputs.', job['if'], name)

    def test_release_publication_keeps_full_ordered_checks(self):
        spec = self.spec('release-please.yml')
        self.assertEqual(spec['concurrency']['cancel-in-progress'], 'false')
        for name in ('application-release-checks', 'chart-release-checks'):
            self.assertNotIn('checks', spec['jobs'][name]['with'], name)


if __name__ == '__main__':
    unittest.main()
