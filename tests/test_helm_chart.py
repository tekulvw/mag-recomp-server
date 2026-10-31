"""Offline Helm graph/contract checks. Requires helm and PyYAML; no cluster access."""
import json
import os
from pathlib import Path
import shutil
import shlex
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


def gameplay_arguments(docs):
    """Execute the rendered launcher with a harmless argv recorder, never a game server."""
    container = next(c for d in docs if d['kind'] == 'Deployment'
                     for c in d['spec']['template']['spec']['containers'] if c['name'] == 'gameplay')
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        recorder = root / 'argv-recorder'
        recorder.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
        recorder.chmod(0o700)
        script = container['args'][0].replace('/usr/local/bin/mag-gameplay', shlex.quote(str(recorder)))
        script = script.replace('/logs/run.', str(root / 'run.'))
        env = {**os.environ, **{e['name']: e['value'] for e in container['env']}}
        result = subprocess.run(container['command'] + [script] + container['args'][1:],
                                env=env, capture_output=True, text=True, timeout=3)
        if result.returncode:
            raise AssertionError(result.stderr)
        return result.stdout.splitlines()


class ChartTests(unittest.TestCase):
    def test_optional_external_bot_names_path(self):
        args = gameplay_arguments(render({'data': {'files': {'botNames': 'operators/bot-names.json'}}}))
        self.assertEqual(args.count('--bot-names'), 1)
        self.assertEqual(args[args.index('--bot-names') + 1], '/data/operators/bot-names.json')
        self.assertNotIn('--bot-names', gameplay_arguments(render()))
        for override in ('--bot-names', '--bot-names=/data/override.json'):
            render({'data': {'gameplayExtraArgs': [override]}}, ok=False)
        for filename in ('../names.json', '/names.json', 'bad name.json'):
            render({'data': {'files': {'botNames': filename}}}, ok=False)

    def test_default_retail_startup_contract(self):
        docs = render()
        service = next(d for d in docs if d['kind'] == 'Service' and d['metadata']['name'].endswith('-medius'))
        self.assertEqual(next(p for p in service['spec']['ports'] if p['name'] == 'maps-auth')['port'], 10073)
        args = gameplay_arguments(docs)
        expected = {
            '--application-id': '20231',
            '--bind': '0.0.0.0:31000',
            '--item-catalog': '/data/retail-items.json',
            '--spawn-catalog': '/data/retail-sver-sup-spawns.json',
            '--wire-profiles': '/data/wire-profiles.json',
            '--skill-catalog': '/data/retail-skills.json',
            '--character-blast-rules': '/data/character-blast-rules.json',
            '--bootstrap-loadouts': '/data/retail-all-factions-bootstrap-loadouts.json',
            '--retail-combat-metadata': '/data/retail-combat.json',
            '--retail-armory-catalog': '/data/retail-armory.json',
            '--fall-damage-rules': '/data/retail-falling-damage.json',
        }
        self.assertEqual(len(args), 2 * (len(expected) + 1))  # Plus the fresh event log.
        for flag, value in expected.items():
            self.assertEqual(args.count(flag), 1)
            self.assertEqual(args[args.index(flag) + 1], value)
        self.assertNotIn('--structure-catalog', args)
        self.assertNotIn('--bunker-catalog', args)
        self.assertNotIn('--bunker-exit-policy', args)
        containers = {c['name']: c for d in docs if d['kind'] == 'Deployment'
                      for c in d['spec']['template']['spec']['containers']}
        self.assertEqual(containers['gameplay']['image'], 'ghcr.io/tekulvw/mag-recomp-server-game:0.2.0')
        self.assertEqual(containers['medius']['image'], 'ghcr.io/tekulvw/mag-recomp-server-medius:0.1.0')
        config = next(d['data'] for d in docs if d['kind'] == 'ConfigMap' and 'main.json' in d.get('data', {}))
        for role in ('main', 'maps'):
            settings = json.loads(config[role + '.json'])
            self.assertEqual((settings['MagApplicationId'], settings['MagProtocolVersion'], settings['MagGameType']), (20231, 1740, 3))
            for key in ('MagItemCatalogPath', 'MagLoadoutDefaultsPath', 'MagSkillCatalogPath', 'MagDevelopmentProgressionPath'):
                self.assertNotIn(key, settings)
        for role in ('medius', 'maps'):
            env = {e['name']: e.get('value') for e in containers[role]['env']}
            self.assertEqual(env['MAG_RETAIL_SKILL_CATALOG'], '/data/retail-skill-purchases.json')
            self.assertEqual(env['MAG_RETAIL_ARMORY_CATALOG'], '/data/retail-armory.json')
            for name in ('MAG_TRAINING_COMPLETION_CATALOG', 'MAG_PROGRESSION_CATALOG', 'MAG_SKILL_CATALOG', 'MAG_LOADOUT_DEFAULTS'):
                self.assertNotIn(name, env)

    def test_explicit_legacy_profile_with_old_images(self):
        for tag in ('0.1.0', '0.1.1'):
            docs = render({'runtime': {'applicationId': 20230, 'protocolVersion': 1725, 'gameType': 2},
                           'images': {'gameplay': {'tag': tag}},
                           'data': {'files': {'items': 'items.json', 'spawns': 'spawns.json', 'skills': 'skills.json', 'trainingCompletion': 'training-completion.json', 'progression': 'progression.json', 'characterBlastRules': None, 'bootstrapLoadouts': None, 'retailCombatMetadata': None, 'retailArmory': None, 'fallDamageRules': None, 'retailSkills': None}}})
            args = gameplay_arguments(docs)
            self.assertNotIn('--application-id', args)
            self.assertNotIn('--retail-armory-catalog', args)
            self.assertNotIn('--bootstrap-loadouts', args)
            self.assertNotIn('--fall-damage-rules', args)
            self.assertEqual(args[args.index('--skill-catalog') + 1], '/data/skills.json')
            medius = next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-medius'))
            for c in medius['spec']['template']['spec']['containers']:
                env = {e['name']: e.get('value') for e in c['env']}
                self.assertNotIn('MAG_RETAIL_SKILL_CATALOG', env)
                self.assertIn('MAG_TRAINING_COMPLETION_CATALOG', env)
            config = next(d['data'] for d in docs if d['kind'] == 'ConfigMap' and 'main.json' in d.get('data', {}))
            self.assertEqual(json.loads(config['main.json'])['MagLoadoutDefaultsPath'], '/data/loadout-defaults.json')

    def test_optional_retail_progression_inputs(self):
        docs = render({'data': {'files': {'trainingCompletion': 'retail-progression.json', 'progression': 'retail-progression.json'}}})
        medius = next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-medius'))
        for c in medius['spec']['template']['spec']['containers']:
            env = {e['name']: e.get('value') for e in c['env']}
            self.assertEqual(env['MAG_TRAINING_COMPLETION_CATALOG'], '/data/retail-progression.json')
            self.assertEqual(env['MAG_PROGRESSION_CATALOG'], '/data/retail-progression.json')
            self.assertEqual(env['MAG_RETAIL_SKILL_CATALOG'], '/data/retail-skill-purchases.json')
            self.assertEqual(env['MAG_RETAIL_ARMORY_CATALOG'], '/data/retail-armory.json')
        docs = render({'data': {'files': {'trainingCompletion': '', 'progression': ''}}})
        for d in (d for d in docs if d['kind'] == 'Deployment'):
            for c in d['spec']['template']['spec']['containers']:
                names = {e['name'] for e in c['env']}
                self.assertNotIn('MAG_TRAINING_COMPLETION_CATALOG', names)
                self.assertNotIn('MAG_PROGRESSION_CATALOG', names)

    def test_retail_catalog_and_argument_guards(self):
        for key in ('items', 'spawns', 'wireProfiles', 'skills', 'characterBlastRules', 'bootstrapLoadouts', 'retailCombatMetadata', 'retailArmory', 'fallDamageRules', 'retailSkills'):
            with self.subTest(key=key):
                render({'data': {'files': {key: None}}}, ok=False)
        for tag in ('0.1.0', '0.1.1'):
            error = render({'images': {'gameplay': {'tag': tag}}}, ok=False)
            self.assertIn('retail gameplay requires', error)
        for flag in ('--application-id', '--bind', '--log', '--item-catalog', '--spawn-catalog', '--wire-profiles', '--skill-catalog', '--character-blast-rules', '--bootstrap-loadouts', '--retail-combat-metadata', '--retail-armory-catalog', '--fall-damage-rules', '--structure-catalog', '--bunker-catalog', '--bunker-exit-policy', '--validate-catalogs'):
            with self.subTest(flag=flag):
                render({'data': {'gameplayExtraArgs': [flag, '/data/override.json']}}, ok=False)
                render({'data': {'gameplayExtraArgs': [flag + '=/data/override.json']}}, ok=False)
        args = gameplay_arguments(render({'data': {'gameplayExtraArgs': ['--collision-world', '/data/collision-world.json']}}))
        self.assertEqual(args[-2:], ['--collision-world', '/data/collision-world.json'])
        # Custom builders and digest pins are not judged by an unrelated numeric tag.
        render({'images': {'gameplay': {'repository': 'registry.example.invalid/custom-game', 'tag': '0.1.0'}}})
        pinned = render({'images': {'gameplay': {'tag': '0.1.0', 'digest': 'sha256:' + 'a' * 64}}})
        image = next(c['image'] for d in pinned if d['kind'] == 'Deployment'
                     for c in d['spec']['template']['spec']['containers'] if c['name'] == 'gameplay')
        self.assertEqual(image, 'ghcr.io/tekulvw/mag-recomp-server-game@sha256:' + 'a' * 64)

    def test_nonconnecting_listener_probes(self):
        containers = {c['name']: c for d in render() if d['kind'] == 'Deployment'
                      for c in d['spec']['template']['spec']['containers']}
        for role, port in {'gameplay': 31000, 'medius': 10475, 'maps': 10471}.items():
            for name in ('startupProbe', 'readinessProbe'):
                probe = containers[role][name]
                self.assertNotIn('tcpSocket', probe)
                command = probe['exec']['command']
                self.assertEqual(command[:2], ['/bin/sh', '-ec'])
                self.assertEqual(command[3:], ['socket-listening', f'{port:04X}'])
                self.assertIn('/proc/net/tcp /proc/net/tcp6', command[2])
            self.assertEqual(containers[role]['startupProbe']['periodSeconds'], 2)
            self.assertEqual(containers[role]['startupProbe']['failureThreshold'], 90)
            self.assertEqual(containers[role]['readinessProbe']['periodSeconds'], 5)

    def test_listener_probe_table_semantics(self):
        containers = {c['name']: c for d in render() if d['kind'] == 'Deployment'
                      for c in d['spec']['template']['spec']['containers']}
        # Independently constructed Linux proc table rows, with no real service/data.
        header = '  sl  local_address rem_address   st tx_queue rx_queue\n'
        def row(address, port, state='0A'):
            return f'  0: {address}:{port:04X} 00000000:0000 {state} 00000000:00000000\n'
        for role, port in {'gameplay': 31000, 'medius': 10475, 'maps': 10471}.items():
            command = containers[role]['readinessProbe']['exec']['command']
            with tempfile.TemporaryDirectory() as temporary:
                ipv4, ipv6 = (Path(temporary) / name for name in ('tcp', 'tcp6'))
                # Only replace the proc table paths; execute the actual rendered command.
                script = command[2].replace('/proc/net/tcp /proc/net/tcp6', f'"{ipv4}" "{ipv6}"')
                def run(v4, v6):
                    for file, text in ((ipv4, v4), (ipv6, v6)):
                        if text is None:
                            file.unlink(missing_ok=True)
                        else:
                            file.write_text(header + text)
                    return subprocess.run(command[:2] + [script] + command[3:], capture_output=True, timeout=2)
                for v4, v6 in ((row('00000000', port), None),
                               ('', row('0' * 32, port)),
                               (None, row('0' * 32, port))):
                    self.assertEqual(run(v4, v6).returncode, 0, role)
                for v4, v6 in (('', ''), (None, None),
                               (row('00000000', port, '01'), ''),
                               (row('00000000', port + 1), ''),
                               (row('00000000', port - 1), ''),
                               ('', row('0' * 32, port, '06'))):
                    self.assertNotEqual(run(v4, v6).returncode, 0, role)

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
            self.assertEqual(env['MAG_RETAIL_SKILL_CATALOG'], '/data/retail-skill-purchases.json')
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
