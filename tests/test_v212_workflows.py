"""Validate the integration publication graph without contacting GitHub or a registry."""
import json
from pathlib import Path
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[1]


def read_workflow(name):
    return yaml.load((ROOT / '.github/workflows' / name).read_text(), Loader=yaml.BaseLoader)


class V212WorkflowsTests(unittest.TestCase):
    def test_candidates_only_prepare_prs_against_the_integration_branch(self):
        spec = read_workflow('v212-release-please.yml')
        self.assertEqual(spec['on']['push']['branches'], ['codex/retail-v2.12'])
        job = spec['jobs']['candidate']
        self.assertEqual(job['if'], "github.ref == 'refs/heads/codex/retail-v2.12'")
        step = next(s for s in job['steps'] if s.get('uses', '').startswith('googleapis/release-please-action@'))
        self.assertEqual(step['with']['target-branch'], 'codex/retail-v2.12')
        self.assertEqual(step['with']['skip-github-release'], 'true')
        self.assertEqual(set(spec['jobs']), {'candidate'})
        config = json.loads((ROOT / step['with']['config-file']).read_text())
        stable = json.loads((ROOT / 'release-please-config.json').read_text())
        self.assertEqual(config['versioning'], 'prerelease')
        self.assertTrue(config['prerelease'])
        self.assertEqual(config['prerelease-type'], 'v212')
        self.assertEqual(set(config['packages']), set(stable['packages']))
        for path, package in config['packages'].items():
            self.assertEqual(package['component'], stable['packages'][path]['component'] + '-212')
        self.assertNotEqual(step['with']['manifest-file'], '.release-please-manifest.json')
        self.assertEqual(set(json.loads((ROOT / step['with']['manifest-file']).read_text())), set(config['packages']))

    def test_images_require_checks_for_the_same_branch_commit(self):
        spec = read_workflow('v212-integration-images.yml')
        self.assertEqual(set(spec['on']), {'workflow_call'})
        dispatch = read_workflow('main-images.yml')['jobs']['integration-v212']
        self.assertEqual(dispatch['if'], "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/codex/retail-v2.12'")
        self.assertEqual(dispatch['uses'], './.github/workflows/v212-integration-images.yml')
        self.assertEqual(spec['permissions'], {})
        checks, build = spec['jobs']['checks'], spec['jobs']['build']
        self.assertEqual(checks['if'], "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/codex/retail-v2.12'")
        self.assertEqual(checks['uses'], './.github/workflows/check.yml')
        self.assertEqual(checks['with']['ref'], '${{ github.sha }}')
        self.assertEqual(build['needs'], 'checks')
        self.assertEqual(build['permissions'], {'contents': 'read', 'packages': 'write'})
        self.assertEqual(build['strategy']['matrix']['include'], [
            {'context': 'horizon', 'suffix': 'medius'},
            {'context': 'gameplay', 'suffix': 'game'},
            {'context': 'portal', 'suffix': 'portal'}])
        checkout = next(s for s in build['steps'] if s.get('uses', '').startswith('actions/checkout@'))
        self.assertEqual(checkout['with']['ref'], '${{ github.sha }}')
        image = next(s for s in build['steps'] if s.get('uses', '').startswith('docker/build-push-action@'))
        self.assertEqual(image['with']['tags'], '${{ steps.source.outputs.image }}:v212-${{ github.sha }}-${{ github.run_id }}-${{ github.run_attempt }}')
        self.assertNotIn('cache-to', image['with'])
        self.assertEqual(image['with']['push'], 'true')
        self.assertIn('${{ steps.image.outputs.digest }}', build['steps'][-1]['env']['DIGEST'])


if __name__ == '__main__':
    unittest.main()
