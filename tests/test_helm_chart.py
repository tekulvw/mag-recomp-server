"""Offline Helm graph/contract checks. Requires helm and PyYAML; no cluster access."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import yaml

CHART = Path(__file__).resolve().parents[1] / 'charts' / 'mag-recomp-server'
HELM = shutil.which('helm') or '/opt/homebrew/bin/helm'
BASE = {'network': {'advertisedIPv4': '203.0.113.10', 'authDns': 'auth.example.invalid'}, 'runtime': {'existingSecret': 'runtime-keys'}}


def render(extra=None, ok=True):
    values = dict(BASE)
    for key, value in (extra or {}).items():
        values[key] = {**values.get(key, {}), **value} if isinstance(value, dict) else value
    with tempfile.NamedTemporaryFile(mode='w', suffix='.json') as f:
        json.dump(values, f); f.flush()
        result = subprocess.run([HELM, 'template', 'test', str(CHART), '-f', f.name], capture_output=True, text=True)
    if ok:
        if result.returncode: raise AssertionError(result.stderr)
        return list(yaml.safe_load_all(result.stdout))
    if result.returncode == 0: raise AssertionError('invalid values unexpectedly rendered')
    return result.stderr


class ChartTests(unittest.TestCase):
    def test_default_graph(self):
        docs = render()
        deployments = [d for d in docs if d['kind'] == 'Deployment']
        self.assertEqual(len(deployments), 2)
        self.assertEqual(sorted(len(d['spec']['template']['spec']['containers']) for d in deployments), [1, 2])
        for d in deployments:
            self.assertEqual(d['spec']['replicas'], 1)
            self.assertEqual(d['spec']['strategy']['type'], 'Recreate')
            p = d['spec']['template']['spec']
            self.assertFalse(p['automountServiceAccountToken'])
            self.assertEqual(p['securityContext']['runAsUser'], 10001)
            for c in p['containers']:
                data = next(v for v in c['volumeMounts'] if v['name'] == 'data')
                self.assertTrue(data['readOnly'])
        self.assertFalse(any(d['kind'] == 'Secret' for d in docs))
        services = {d['metadata']['name']: d for d in docs if d['kind'] == 'Service'}
        external = next(d for d in services.values() if d['spec']['type'] == 'LoadBalancer')
        self.assertEqual({p['protocol'] for p in external['spec']['ports']}, {'TCP', 'UDP'})
        private = next(d for d in services.values() if d['spec']['type'] == 'ClusterIP')
        self.assertEqual(private['spec']['ports'][0]['port'], 31000)
        medius = next(d for d in deployments if len(d['spec']['template']['spec']['containers']) == 2)
        containers = medius['spec']['template']['spec']['containers']
        main, maps = (next(c for c in containers if c['name'] == n) for n in ['medius', 'maps'])
        self.assertFalse(next(m for m in main['volumeMounts'] if m['name'] == 'accounts')['readOnly'])
        self.assertTrue(next(m for m in maps['volumeMounts'] if m['name'] == 'accounts')['readOnly'])
        ports = [p['containerPort'] for c in containers for p in c['ports'] if p['protocol'] == 'TCP']
        self.assertEqual(len(ports), len(set(ports)))

    def test_original_maps_port_mapping(self):
        docs = render({'network': {'mapsPort': 10071}})
        service = next(d for d in docs if d['kind'] == 'Service' and d['metadata']['name'].endswith('-medius'))
        maps = next(p for p in service['spec']['ports'] if p['name'] == 'maps-auth')
        self.assertEqual(maps['port'], 10071)
        self.assertEqual(maps['targetPort'], 'maps-auth')

    def test_s3_graph(self):
        docs = render({'data': {'mode': 's3', 's3': {'bucket': 'private-runtime', 'key': 'bundles/sample.tar.gz', 'sha256': 'a'*64, 'existingSecret': 'object-store'}}})
        for d in (d for d in docs if d['kind'] == 'Deployment'):
            p = d['spec']['template']['spec']
            self.assertIn('emptyDir', next(v for v in p['volumes'] if v['name'] == 'data'))
            init = {c['name']: c for c in p['initContainers']}
            self.assertIn('fetch-data', init)
            self.assertIn('verify-data', init)
            self.assertEqual(init['fetch-data']['envFrom'], [{'secretRef': {'name': 'object-store'}}])
            self.assertIn('a'*64, init['verify-data']['args'])
        scripts = next(d for d in docs if d['kind'] == 'ConfigMap' and 'data_bundle.py' in d.get('data', {}))
        self.assertIn('unpack', scripts['data']['data_bundle.py'])

    def test_runtime_config_contract(self):
        docs = render()
        config = next(d for d in docs if d['kind'] == 'ConfigMap' and 'main.json' in d.get('data', {}))['data']
        main, maps = json.loads(config['main.json']), json.loads(config['maps.json'])
        self.assertTrue(main['EnableMagMasAdapter'])
        self.assertFalse(main['EnableMagPluginAdapter'])
        self.assertTrue(maps['EnableMagPluginAdapter'])
        self.assertFalse(maps['EnableMagMasAdapter'])
        self.assertTrue(maps['EnableMagCharacterStorage'])
        for role in (main, maps):
            self.assertTrue(role['EnableMagDevelopmentAuth'])
        deployment = next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-medius'))
        for c in deployment['spec']['template']['spec']['containers']:
            env = {e['name']: e.get('value') for e in c['env']}
            self.assertNotIn('MAG_RETAIL_SKILL_CATALOG', env)
            self.assertEqual(env['HORIZON_SCERT_MAS_CERTIFICATE_PATH'], '/runtime-secrets/scert-mas-certificate.bin')
            self.assertEqual(env['HORIZON_SCERT_MLS_CERTIFICATE_PATH'], '/runtime-secrets/scert-mls-certificate.bin')
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            public, private, output = (work / p for p in ['public', 'private', 'output'])
            public.mkdir(); private.mkdir(); output.mkdir()
            for role in ['main', 'maps']:
                (public / (role + '.json')).write_text(config[role + '.json'])
            # Tiny mathematical values exercise only the JSON merge; never used to serve traffic.
            keys = {name: {'n': '17', 'e': '3', 'd': '11'} for name in ['MPSKey', 'DefaultKey']}
            (private / 'medius-keys.json').write_text(json.dumps(keys))
            for name in ['scert-mas-certificate.bin', 'scert-mls-certificate.bin']:
                (private / name).write_bytes(b'\x00' * 468)
            command = [os.sys.executable, str(CHART / 'files/init_config.py'), str(public), str(private), str(output)]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads((output/'main/medius.json').read_text())['MPSKey'], keys['MPSKey'])
            self.assertEqual((output/'main/medius.json').stat().st_mode & 0o777, 0o600)
            self.assertNotIn('MPSKey', json.loads(config['main.json']))
            (private / 'scert-mas-certificate.bin').write_bytes(b'\x00' * 467)
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            (private / 'scert-mas-certificate.bin').write_bytes(b'\x00' * 468)
            (private / 'medius-keys.json').write_text(json.dumps({**keys, 'MASPort': 1234}))
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)

    def test_optional_dns(self):
        config = {'dns': {'enabled': True, 'answerIPv4': '203.0.113.10', 'hostnames': ['original.example.invalid'], 'service': {'loadBalancerSourceRanges': ['203.0.113.0/24']}}}
        docs = render(config)
        dns = next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-dns'))
        self.assertEqual(dns['spec']['replicas'], 1)
        service = next(d for d in docs if d['kind'] == 'Service' and d['metadata']['name'].endswith('-dns'))
        self.assertEqual({p['protocol'] for p in service['spec']['ports']}, {'TCP', 'UDP'})
        self.assertEqual(service['spec']['loadBalancerSourceRanges'], ['203.0.113.0/24'])
        core = next(d for d in docs if d['kind'] == 'ConfigMap' and 'Corefile' in d.get('data', {}))['data']['Corefile']
        self.assertIn('203.0.113.10 original.example.invalid', core)
        self.assertIn('fallthrough', core)
        self.assertIn('forward . /etc/resolv.conf', core)
        self.assertIn('no_reverse', core)
        self.assertIn('response clear aa', core)
        self.assertIn('response set ra', core)
        base = config['dns']
        cases = [
            {'hostnames': []},
            {'hostnames': ['*.example.invalid']},
            {'hostnames': ['https://example.invalid/path']},
            {'hostnames': ['a' * 64 + '.example.invalid']},
            {'answerIPv4': '999.1.1.1'},
            {'upstreams': ['999.999.999.999:99999']},
            {'service': {'loadBalancerSourceRanges': []}},
        ]
        for bad in cases:
            render({'dns': {**base, **bad}}, ok=False)


    def test_invalid_values(self):
        cases = [
            {'network': {'advertisedIPv4': ''}},
            {'network': {'mapsPort': 10475}},
            {'network': {'mapsPort': 0}},
            {'network': {'advertisedIPv4': '999.1.1.1'}},
            {'runtime': {'existingSecret': ''}},
            {'runtime': {'minimumPlayers': 8, 'maximumPlayers': 1}},
            {'replicaCount': 2},
            {'data': {'mode': 'inline'}},
            {'data': {'mode': 's3', 's3': {'bucket': 'b', 'key': 'k', 'sha256': 'bad', 'existingSecret': 's'}}},
            {'data': {'mode': 's3', 's3': {'bucket': 'b', 'key': 'k', 'sha256': 'a'*64}}},
            {'data': {'existingClaim': ''}},
            {'runtime': {'privateKey': 'never-inline'}},
        ]
        for case in cases:
            with self.subTest(case=case): render(case, ok=False)

    def test_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run([HELM, 'package', str(CHART), '--destination', tmp], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            version = yaml.safe_load((CHART / 'Chart.yaml').read_text())['version']
            self.assertTrue((Path(tmp)/f'mag-recomp-server-{version}.tgz').exists())


if __name__ == '__main__': unittest.main()
