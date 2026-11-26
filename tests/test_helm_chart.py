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


def gameplay_arguments(docs, deployment=None):
    """Execute the rendered launcher with a harmless argv recorder, never a game server."""
    container = next(c for d in docs if d['kind'] == 'Deployment'
                     and deployment in (None, d['metadata']['name'])
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
    def test_news_uses_selected_gameplay_image_and_rolls_with_it(self):
        def news_contract(overrides=None):
            docs = render(overrides)
            deployments = {d['metadata']['name'].rsplit('-', 1)[-1]: d for d in docs if d['kind'] == 'Deployment'}
            pod = deployments['medius']['spec']['template']
            spec = pod['spec']
            self.assertIn('prepare-news', [c['name'] for c in spec['initContainers']], 'deployed gameplay notes are not bundled')
            copier = next(c for c in spec['initContainers'] if c['name'] == 'prepare-news')
            worker = deployments['gameplay']['spec']['template']['spec']['containers'][0]
            self.assertEqual(copier['image'], worker['image'])
            for container in spec['containers']:
                env = {e['name']: e.get('value') for e in container['env']}
                self.assertEqual(env['MAG_GAMEPLAY_NEWS_PATH'], '/news/gameplay-CHANGELOG.md')
                mount = next(m for m in container['volumeMounts'] if m['name'] == 'news')
                self.assertTrue(mount['readOnly'])
            self.assertIn('emptyDir', next(v for v in spec['volumes'] if v['name'] == 'news'))
            return pod['metadata']['annotations']['checksum/news-image'], copier

        old, copier = news_contract()
        new, _ = news_contract({'images': {'gameplay': {'tag': '9.8.7'}}})
        self.assertNotEqual(old, new)
        digest, _ = news_contract({'images': {'gameplay': {'digest': 'sha256:' + 'a' * 64}}})
        self.assertNotEqual(old, digest)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'bundled.md'
            output = root / 'news'
            output.mkdir()
            script = copier['args'][0].replace('/usr/share/mag-recomp-server/gameplay-CHANGELOG.md', str(source))
            script = script.replace('/news', str(output))
            # Older gameplay images have no bundled notes; no fabricated gameplay NEWS.
            subprocess.run(copier['command'] + [script], check=True, timeout=3)
            self.assertEqual(list(output.iterdir()), [])
            source.write_text('## 9.8.7 (2026-11-02)\n* Synthetic gameplay fix\n')
            subprocess.run(copier['command'] + [script], check=True, timeout=3)
            self.assertEqual((output / 'gameplay-CHANGELOG.md').read_text(), source.read_text())

    def test_optional_external_bot_names_path(self):
        args = gameplay_arguments(render({'data': {'files': {'botNames': 'operators/bot-names.json'}}}))
        self.assertEqual(args.count('--bot-names'), 1)
        self.assertEqual(args[args.index('--bot-names') + 1], '/data/operators/bot-names.json')
        self.assertNotIn('--bot-names', gameplay_arguments(render()))
        for override in ('--bot-names', '--bot-names=/data/override.json'):
            render({'data': {'gameplayExtraArgs': [override]}}, ok=False)
        for filename in ('../names.json', '/names.json', 'bad name.json'):
            render({'data': {'files': {'botNames': filename}}}, ok=False)

    def test_optional_gameplay_log_rotation(self):
        args = gameplay_arguments(render({'logs': {'rotation': {'segmentBytes': 67108864, 'retentionSeconds': 21600}}}))
        self.assertEqual(args[args.index('--log-segment-bytes') + 1], '67108864')
        self.assertEqual(args[args.index('--log-retention-s') + 1], '21600')
        segment_only = gameplay_arguments(render({'logs': {'rotation': {'segmentBytes': 1048576}}}))
        self.assertIn('--log-segment-bytes', segment_only)
        self.assertNotIn('--log-retention-s', segment_only)
        default = gameplay_arguments(render())
        self.assertNotIn('--log-segment-bytes', default)
        self.assertNotIn('--log-retention-s', default)
        self.assertIn('requires logs.rotation.segmentBytes',
                      render({'logs': {'rotation': {'retentionSeconds': 60}}}, ok=False))
        render({'logs': {'rotation': {'segmentBytes': 1024}}}, ok=False)
        for flag in ('--log-segment-bytes', '--log-retention-s'):
            render({'data': {'gameplayExtraArgs': [flag, '60']}}, ok=False)

    def test_optional_portal_tickets(self):
        def medius_env(docs):
            return [{e['name']: e.get('value') for e in c.get('env', [])}
                    for d in docs if d['kind'] == 'Deployment'
                    for c in d['spec']['template']['spec']['containers'] if c['name'] in ('medius', 'maps')]
        names = ('MAG_PORTAL_TICKETS', 'MAG_PORTAL_TICKET_AUDIENCE', 'MAG_PORTAL_TICKET_KEYS')
        for off in (None, {'portalTickets': {'mode': 'off', 'audience': 'play.example.test', 'keys': {'1': 'QUJD'}}}):
            envs = medius_env(render(off))
            self.assertEqual(len(envs), 2)
            self.assertTrue(all(name not in env for env in envs for name in names))
        key = 'MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEXvgof3Vza4+kYlYDHvNkzoWnOiYVUPKfTwLOxfsFFv2tfRNAngFLgL48OzLv2VMJ+iYgvwndYncrd9TFMMLxrw=='
        envs = medius_env(render({'portalTickets': {'mode': 'required', 'audience': 'play.example.test', 'keys': {'2': key, '1': key}}}))
        self.assertEqual(len(envs), 2)
        for env in envs:
            self.assertEqual(env['MAG_PORTAL_TICKETS'], 'required')
            self.assertEqual(env['MAG_PORTAL_TICKET_AUDIENCE'], 'play.example.test')
            self.assertEqual(env['MAG_PORTAL_TICKET_KEYS'], f'1:{key},2:{key}')
        self.assertIn('keys', render({'portalTickets': {'mode': 'required', 'audience': 'play.example.test'}}, ok=False))
        for invalid in ({'mode': 'required', 'keys': {'1': key}},
                        {'mode': 'required', 'audience': '', 'keys': {'1': key}},
                        {'mode': 'required', 'audience': 'play.example.test', 'keys': {}},
                        {'mode': 'on', 'audience': 'play.example.test', 'keys': {'1': key}},
                        {'mode': 'required', 'audience': 'Play.Example.Test', 'keys': {'1': key}},
                        {'mode': 'required', 'audience': 'play.example.test', 'keys': {'256': key}},
                        {'mode': 'required', 'audience': 'play.example.test', 'keys': {'1': 'not base64!'}},
                        {'mode': 'required', 'audience': 'play.example.test', 'keys': {'1': key}, 'extra': True}):
            render({'portalTickets': invalid}, ok=False)

    @staticmethod
    def policy_allows(policy, port, protocol, namespace):
        """Whether an ingress rule admits port/protocol; namespace 'mag-portal' matches the CI-style selector."""
        for rule in policy['spec']['ingress']:
            sources = rule.get('from')
            if sources is not None and not any(
                    s.get('namespaceSelector', {}).get('matchLabels', {}).get('kubernetes.io/metadata.name') == namespace
                    for s in sources):
                continue
            for entry in rule['ports']:
                if entry.get('protocol', 'TCP') == protocol and entry['port'] <= port <= entry.get('endPort', entry['port']):
                    return True
        return False

    def assert_admin_policy(self, policy, admin_port, selector):
        open_rules = [r for r in policy['spec']['ingress'] if 'from' not in r]
        admin_rules = [r for r in policy['spec']['ingress'] if 'from' in r]
        self.assertEqual(len(open_rules), 1)
        self.assertEqual(admin_rules, [{'from': [{'namespaceSelector': selector}], 'ports': [{'protocol': 'TCP', 'port': admin_port}]}])
        for entry in open_rules[0]['ports']:
            self.assertIn(entry['protocol'], ('TCP', 'UDP'))
            self.assertLessEqual(entry['port'], entry['endPort'])
            self.assertFalse(entry['port'] <= admin_port <= entry['endPort'], 'admin port must not be open to every source')
        for protocol in ('TCP', 'UDP'):
            for number in (1, admin_port - 1, admin_port + 1, 65535, 10476):
                if 1 <= number <= 65535 and number != admin_port:
                    self.assertTrue(self.policy_allows(policy, number, protocol, 'elsewhere'), (number, protocol))
        self.assertFalse(self.policy_allows(policy, admin_port, 'TCP', 'elsewhere'))
        self.assertTrue(self.policy_allows(policy, admin_port, 'TCP', 'mag-portal'))

    def test_optional_admin_api(self):
        def medius(docs):
            deployment = next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-medius'))
            spec = deployment['spec']['template']['spec']
            return {c['name']: c for c in spec['containers']}, spec['volumes']
        names = ('MAG_ADMIN_API_LISTEN', 'MAG_ADMIN_API_SECRET_FILE')
        docs = render()
        containers, volumes = medius(docs)
        for container in containers.values():
            self.assertTrue(all(e['name'] not in names for e in container['env']))
            self.assertNotIn('admin-api', [p['name'] for p in container['ports']])
        self.assertNotIn('admin-api-secret', [v['name'] for v in volumes])
        self.assertFalse(any(d['kind'] == 'NetworkPolicy' or d['metadata']['name'].endswith('-admin-api') for d in docs))

        selector = {'matchLabels': {'kubernetes.io/metadata.name': 'mag-portal'}}
        enabled = {'enabled': True, 'existingSecret': 'mag-admin-api', 'allowedNamespaceSelector': selector}
        docs = render({'adminApi': enabled})
        containers, volumes = medius(docs)
        main, maps = containers['medius'], containers['maps']
        env = {e['name']: e.get('value') for e in main['env']}
        self.assertEqual(env['MAG_ADMIN_API_LISTEN'], '0.0.0.0:10490')
        self.assertEqual(env['MAG_ADMIN_API_SECRET_FILE'], '/admin-api/secret')
        self.assertIn({'name': 'admin-api', 'containerPort': 10490, 'protocol': 'TCP'}, main['ports'])
        self.assertIn({'name': 'admin-api-secret', 'mountPath': '/admin-api', 'readOnly': True}, main['volumeMounts'])
        self.assertTrue(all(e['name'] not in names for e in maps['env']), 'the admin API must never run in MAPS')
        self.assertNotIn('admin-api', [p['name'] for p in maps['ports']])
        self.assertNotIn('admin-api-secret', [m['name'] for m in maps['volumeMounts']])
        secret = next(v for v in volumes if v['name'] == 'admin-api-secret')['secret']
        self.assertEqual(secret['secretName'], 'mag-admin-api')
        self.assertEqual(secret['items'], [{'key': 'admin-api-secret', 'path': 'secret'}])
        self.assertFalse(any(d['kind'] == 'Secret' for d in docs))

        service = next(d for d in docs if d['kind'] == 'Service' and d['metadata']['name'] == 'test-mag-recomp-server-admin-api')
        self.assertEqual(service['spec']['type'], 'ClusterIP')
        self.assertEqual(service['spec']['selector']['app.kubernetes.io/component'], 'medius')
        self.assertEqual(service['spec']['ports'], [{'name': 'admin-api', 'port': 10490, 'targetPort': 'admin-api', 'protocol': 'TCP'}])
        external = next(d for d in docs if d['kind'] == 'Service' and d['spec']['type'] == 'LoadBalancer')
        self.assertNotIn(10490, [p['port'] for p in external['spec']['ports']])
        policy = next(d for d in docs if d['kind'] == 'NetworkPolicy')
        self.assertEqual(policy['spec']['podSelector']['matchLabels']['app.kubernetes.io/component'], 'medius')
        self.assertEqual(policy['spec']['policyTypes'], ['Ingress'])
        self.assert_admin_policy(policy, 10490, selector)
        # Ports outside the chart (for example extra UDP game ports added by kustomize) stay open too.
        self.assertTrue(self.policy_allows(policy, 10476, 'UDP', 'game-namespace'))
        declared = [(p['containerPort'], p['protocol']) for c in containers.values() for p in c['ports'] if p['name'] != 'admin-api']
        self.assertTrue(declared)
        for number, protocol in declared:
            self.assertTrue(self.policy_allows(policy, number, protocol, 'game-namespace'), (number, protocol))
        for edge in (1024, 1025, 65534, 65535):
            edge_docs = render({'adminApi': {**enabled, 'port': edge}})
            self.assert_admin_policy(next(d for d in edge_docs if d['kind'] == 'NetworkPolicy'), edge, selector)

        custom = render({'adminApi': {**enabled, 'port': 18080, 'secretKey': 'token',
                                      'allowedNamespaceSelector': {'matchExpressions': [{'key': 'team', 'operator': 'In', 'values': ['portal']}]}}})
        containers, volumes = medius(custom)
        self.assertEqual({e['name']: e.get('value') for e in containers['medius']['env']}['MAG_ADMIN_API_LISTEN'], '0.0.0.0:18080')
        self.assertEqual(next(v for v in volumes if v['name'] == 'admin-api-secret')['secret']['items'][0]['key'], 'token')

        for invalid in ({'enabled': True, 'existingSecret': 'mag-admin-api'},
                        {**enabled, 'allowedNamespaceSelector': {}},
                        {**enabled, 'allowedNamespaceSelector': {'matchLabels': {}}},
                        {**enabled, 'existingSecret': ''},
                        {**enabled, 'port': 10475},
                        {**enabled, 'port': 80},
                        {**enabled, 'secret': 'never-inline'}):
            with self.subTest(invalid=invalid):
                render({'adminApi': invalid}, ok=False)
        # The template refuses an incomplete configuration even without the schema.
        for invalid, message in (({**enabled, 'allowedNamespaceSelector': {}}, 'allowedNamespaceSelector'),
                                 ({**enabled, 'existingSecret': ''}, 'existingSecret')):
            values = {**BASE, 'adminApi': invalid}
            with tempfile.NamedTemporaryFile(mode='w', suffix='.json') as f:
                json.dump(values, f); f.flush()
                result = subprocess.run([HELM, 'template', 'test', str(CHART), '-f', f.name, '--skip-schema-validation'],
                                        capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(message, result.stderr)

    def test_optional_starting_level(self):
        def medius_env(docs):
            return [{e['name']: e.get('value') for e in c.get('env', [])}
                    for d in docs if d['kind'] == 'Deployment'
                    for c in d['spec']['template']['spec']['containers'] if c['name'] in ('medius', 'maps')]
        self.assertTrue(all('MAG_STARTING_LEVEL' not in env for env in medius_env(render())))
        envs = medius_env(render({'runtime': {'startingLevel': 3}}))
        self.assertTrue(envs and all(env.get('MAG_STARTING_LEVEL') == '3' for env in envs))
        for invalid in (0, 256, '3', [3]):
            render({'runtime': {'startingLevel': invalid}}, ok=False)

    def test_optional_training_level(self):
        def medius_env(docs):
            return [{e['name']: e.get('value') for e in c.get('env', [])}
                    for d in docs if d['kind'] == 'Deployment'
                    for c in d['spec']['template']['spec']['containers'] if c['name'] in ('medius', 'maps')]
        self.assertTrue(all('MAG_TRAINING_LEVEL' not in env for env in medius_env(render())))
        envs = medius_env(render({'runtime': {'trainingLevel': 3}}))
        self.assertTrue(envs and all(env.get('MAG_TRAINING_LEVEL') == '3' and 'MAG_STARTING_LEVEL' not in env for env in envs))
        for invalid in (0, 256, '3', [3]):
            render({'runtime': {'trainingLevel': invalid}}, ok=False)

    def test_optional_reward_ledger_open_match_ttl(self):
        def medius_env(docs):
            return [{e['name']: e.get('value') for e in c.get('env', [])}
                    for d in docs if d['kind'] == 'Deployment'
                    for c in d['spec']['template']['spec']['containers'] if c['name'] in ('medius', 'maps')]
        self.assertTrue(all('MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS' not in env for env in medius_env(render())))
        envs = medius_env(render({'runtime': {'rewardLedgerOpenMatchTtlHours': 48}}))
        self.assertTrue(len(envs) >= 2 and all(env.get('MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS') == '48' for env in envs))
        for invalid in (0, 721, '48', [48]):
            render({'runtime': {'rewardLedgerOpenMatchTtlHours': invalid}}, ok=False)

    def test_optional_initial_spawn_delay_override(self):
        default = gameplay_arguments(render())
        self.assertNotIn('--initial-spawn-delay-ms', default)
        both = gameplay_arguments(render({'runtime': {'initialSpawnDelayMs': 10000}}))
        self.assertEqual(both[both.index('--initial-spawn-delay-ms') + 1], '10000')
        per_team = gameplay_arguments(render({'runtime': {'initialSpawnDelayMs': [10000, 15000]}}))
        self.assertEqual(per_team[per_team.index('--initial-spawn-delay-ms') + 1], '10000,15000')
        for invalid in (0, 120001, [10000], [10000, 0], [1, 2, 3], '10000'):
            render({'runtime': {'initialSpawnDelayMs': invalid}}, ok=False)
        self.assertIn('chart-managed', render(
            {'data': {'gameplayExtraArgs': ['--initial-spawn-delay-ms', '10000']}}, ok=False))

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

    def test_acquisition_mode_reaches_both_medius_roles(self):
        docs = render({'runtime': {'gameType': 1}})
        config = next(d['data'] for d in docs if d['kind'] == 'ConfigMap'
                      and 'main.json' in d.get('data', {}))
        for role in ('main', 'maps'):
            self.assertEqual(json.loads(config[role + '.json'])['MagGameType'], 1)
        for invalid in (-1, 4, '1'):
            render({'runtime': {'gameType': invalid}}, ok=False)

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
            self.assertEqual(init['verify-data']['args'][-1], '--replace-existing')
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

    WORKER_SHA = {'sabotage': 'b' * 64, 'acquisition': 'c' * 64, 'domination': 'd' * 64}

    @classmethod
    def workers(cls):
        """Three synthetic workers; the store location comes from data.s3 unless a worker overrides it."""
        return [
            {'id': 'sabotage', 'matchProfiles': 'sabotage/match-profiles.json',
             'data': {'s3': {'key': 'workers/sabotage.tar.gz', 'sha256': cls.WORKER_SHA['sabotage']}}},
            {'id': 'acquisition', 'name': 'acq', 'matchProfiles': 'acquisition/match-profiles.json',
             'data': {'revision': '2', 's3': {'key': 'workers/acquisition.tar.gz', 'sha256': cls.WORKER_SHA['acquisition'],
                                              'bucket': 'other-bucket', 'forcePathStyle': False},
                      'files': {'spawns': 'acquisition/spawns.json', 'botNames': 'acquisition/names.json'}},
             'extraArgs': ['--party-reservations'], 'minimumPlayers': 1, 'maximumPlayers': 16,
             'minimumPlayersPerTeam': [2, 2], 'resources': {'requests': {'cpu': '1', 'memory': '1Gi'}},
             'nodeSelector': {'example.invalid/pool': 'gameplay'},
             'tolerations': [{'key': 'dedicated', 'operator': 'Equal', 'value': 'gameplay', 'effect': 'NoSchedule'}]},
            {'id': 'domination', 'matchProfiles': 'domination/match-profiles.json', 'emptyDirSizeLimit': '12Gi',
             'data': {'s3': {'key': 'workers/domination.tar.gz', 'sha256': cls.WORKER_SHA['domination']}},
             'navigation': {'s3': {'key': 'navigation/domination.tar.gz', 'sha256': 'e' * 64}, 'emptyDirSizeLimit': '4Gi'},
             'draining': True},
        ]

    @classmethod
    def worker_values(cls, workers=None, **extra):
        values = {'data': {'mode': 's3', 's3': {'endpoint': 'https://objects.example.invalid', 'bucket': 'private-runtime',
                                                'key': 'medius/catalogs.tar.gz', 'sha256': 'a' * 64, 'versionId': 'medius-version',
                                                'existingSecret': 'object-store'}},
                  'gameplayWorkers': cls.workers() if workers is None else workers}
        values.update(extra)
        return values

    @staticmethod
    def medius_pod(docs):
        return next(d for d in docs if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-medius'))['spec']['template']

    def test_no_gameplay_workers_keeps_the_single_gameplay_graph(self):
        def text(extra):
            values = {**BASE, **extra}
            with tempfile.NamedTemporaryFile(mode='w', suffix='.json') as f:
                json.dump(values, f); f.flush()
                result = subprocess.run([HELM, 'template', 'test', str(CHART), '-f', f.name], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout
        s3 = {'data': {'mode': 's3', 's3': {'bucket': 'b', 'key': 'k', 'sha256': 'a' * 64, 'existingSecret': 's'}}}
        for extra in ({}, s3):
            self.assertEqual(text(extra), text({**extra, 'gameplayWorkers': [], 'worldRegistry': {'workerWaitSeconds': 300},
                                                'network': {**BASE['network'], 'udpPortRange': {'start': 10474, 'end': 10474}}}))
        docs = render()
        self.assertEqual([d['metadata']['name'] for d in docs if d['kind'] == 'Deployment'],
                         ['test-mag-recomp-server-gameplay', 'test-mag-recomp-server-medius'])
        self.assertFalse(any(d['metadata']['name'].endswith('-world-registry') for d in docs))
        pod = self.medius_pod(docs)
        self.assertNotIn('wait-for-workers', [c['name'] for c in pod['spec']['initContainers']])
        self.assertFalse(any(k.startswith('checksum/gameplay') or k == 'checksum/world-registry' for k in pod['metadata']['annotations']))
        env = {e['name']: e.get('value') for e in pod['spec']['containers'][0]['env']}
        self.assertEqual(env['MAG_GAMEPLAY_HOST'], 'test-mag-recomp-server-gameplay')
        self.assertNotIn('MAG_WORLD_REGISTRY', env)
        self.assertNotIn('--worker-id', gameplay_arguments(docs))
        scripts = next(d for d in docs if d['kind'] == 'ConfigMap' and d['metadata']['name'].endswith('-scripts'))
        self.assertNotIn('wait_for_workers.py', scripts['data'])

    def test_three_gameplay_workers(self):
        docs = render(self.worker_values(worldRegistry={'partyInviteSeconds': 30}))
        name = 'test-mag-recomp-server'
        suffixes = {'sabotage': 'sabotage', 'acquisition': 'acq', 'domination': 'domination'}
        deployments = {d['metadata']['name']: d for d in docs if d['kind'] == 'Deployment'}
        self.assertEqual(sorted(deployments), sorted([f'{name}-gameplay-{s}' for s in suffixes.values()] + [f'{name}-medius']))
        services = {d['metadata']['name']: d for d in docs if d['kind'] == 'Service'}
        self.assertEqual(sorted(n for n, d in services.items() if d['spec']['type'] == 'ClusterIP'),
                         sorted(f'{name}-gameplay-{s}' for s in suffixes.values()))
        workers = {w['id']: w for w in self.workers()}
        for worker_id, suffix in suffixes.items():
            with self.subTest(worker=worker_id):
                deployment = deployments[f'{name}-gameplay-{suffix}']
                service = services[f'{name}-gameplay-{suffix}']
                pod = deployment['spec']['template']
                labels = pod['metadata']['labels']
                self.assertEqual(deployment['spec']['selector']['matchLabels'], {k: labels[k] for k in service['spec']['selector']})
                self.assertEqual(service['spec']['selector']['app.kubernetes.io/component'], f'gameplay-{suffix}')
                self.assertEqual(service['spec']['ports'], [{'name': 'bridge', 'port': 31000, 'targetPort': 'bridge', 'protocol': 'TCP'}])
                self.assertEqual((deployment['spec']['replicas'], deployment['spec']['strategy']['type']), (1, 'Recreate'))
                args = gameplay_arguments(docs, deployment['metadata']['name'])
                self.assertEqual(args[args.index('--worker-id') + 1], worker_id)
                self.assertEqual(args[args.index('--match-profiles') + 1], f'/data/{worker_id}/match-profiles.json')
                self.assertEqual(args[args.index('--application-id') + 1], '20231')
                self.assertEqual(args[args.index('--bind') + 1], '0.0.0.0:31000')
                spec = pod['spec']
                init = {c['name']: c for c in spec['initContainers']}
                fetch = {e['name']: e['value'] for e in init['fetch-data']['env']}
                self.assertEqual(fetch['DATA_KEY'], f'workers/{worker_id}.tar.gz')
                self.assertEqual(fetch['DATA_VERSION'], '', 'a worker must never inherit the Medius object version')
                self.assertEqual(fetch['S3_ENDPOINT'], 'https://objects.example.invalid')
                self.assertEqual(init['fetch-data']['envFrom'], [{'secretRef': {'name': 'object-store'}}])
                self.assertIn(self.WORKER_SHA[worker_id], init['verify-data']['args'])
                self.assertNotIn('a' * 64, init['verify-data']['args'])
                volumes = {v['name']: v for v in spec['volumes']}
                self.assertEqual(volumes['data']['emptyDir']['sizeLimit'], workers[worker_id].get('emptyDirSizeLimit', '8Gi'))
                container = spec['containers'][0]
                self.assertTrue(next(m for m in container['volumeMounts'] if m['name'] == 'data')['readOnly'])
        acquisition = deployments[f'{name}-gameplay-acq']['spec']['template']
        fetch = {e['name']: e['value'] for e in next(c for c in acquisition['spec']['initContainers'] if c['name'] == 'fetch-data')['env']}
        self.assertEqual((fetch['DATA_BUCKET'], fetch['ADDRESSING_STYLE']), ('other-bucket', 'virtual'))
        self.assertEqual(acquisition['metadata']['annotations']['data-revision'], '2')
        args = gameplay_arguments(docs, f'{name}-gameplay-acq')
        self.assertEqual(args[args.index('--spawn-catalog') + 1], '/data/acquisition/spawns.json')
        self.assertEqual(args[args.index('--bot-names') + 1], '/data/acquisition/names.json')
        self.assertEqual(args[args.index('--item-catalog') + 1], '/data/retail-items.json')
        self.assertEqual(args[-1], '--party-reservations')
        self.assertEqual(acquisition['spec']['containers'][0]['resources'], {'requests': {'cpu': '1', 'memory': '1Gi'}})
        self.assertEqual(acquisition['spec']['nodeSelector'], {'example.invalid/pool': 'gameplay'})
        self.assertEqual(acquisition['spec']['tolerations'][0]['key'], 'dedicated')
        sabotage = deployments[f'{name}-gameplay-sabotage']['spec']['template']['spec']
        self.assertEqual(sabotage['nodeSelector'], {'kubernetes.io/arch': 'amd64'})
        self.assertNotIn('tolerations', sabotage)
        self.assertEqual(sabotage['containers'][0]['resources']['limits'], {'memory': '2Gi'})
        self.assertNotIn('navigation', [v['name'] for v in sabotage['volumes']])
        domination = deployments[f'{name}-gameplay-domination']['spec']['template']
        init = {c['name']: c for c in domination['spec']['initContainers']}
        self.assertEqual({e['name']: e['value'] for e in init['fetch-navigation']['env']}['DATA_KEY'], 'navigation/domination.tar.gz')
        self.assertIn('e' * 64, init['verify-navigation']['args'])
        self.assertIn({'name': 'navigation', 'mountPath': '/catalogs'}, init['verify-navigation']['volumeMounts'])
        self.assertIn({'name': 'navigation', 'mountPath': '/navigation', 'readOnly': True}, domination['spec']['containers'][0]['volumeMounts'])
        volumes = {v['name']: v for v in domination['spec']['volumes']}
        self.assertEqual(volumes['navigation']['emptyDir']['sizeLimit'], '4Gi')
        self.assertEqual(volumes['navigation-archive']['emptyDir']['sizeLimit'], '4Gi')

        # Medius keeps its own bundle and reads the registry; client_address follows MAG_ADVERTISE_IPV4.
        pod = self.medius_pod(docs)
        init = {c['name']: c for c in pod['spec']['initContainers']}
        self.assertIn('a' * 64, init['verify-data']['args'])
        self.assertEqual({e['name']: e['value'] for e in init['fetch-data']['env']}['DATA_VERSION'], 'medius-version')
        containers = {c['name']: c for c in pod['spec']['containers']}
        env = {e['name']: e.get('value') for e in containers['medius']['env']}
        self.assertEqual(env['MAG_WORLD_REGISTRY'], '/world-registry/world-registry.json')
        self.assertEqual(env['MAG_LOCAL_DME'], '1')
        self.assertEqual(env['MAG_ADVERTISE_IPV4'], '203.0.113.10')
        self.assertNotIn('MAG_GAMEPLAY_HOST', env)
        self.assertIn({'name': 'world-registry', 'mountPath': '/world-registry', 'readOnly': True}, containers['medius']['volumeMounts'])
        self.assertNotIn('world-registry', [m['name'] for m in containers['maps']['volumeMounts']])
        self.assertNotIn('MAG_WORLD_REGISTRY', {e['name'] for e in containers['maps']['env']})
        volume = next(v for v in pod['spec']['volumes'] if v['name'] == 'world-registry')
        self.assertEqual(volume, {'name': 'world-registry', 'configMap': {'name': f'{name}-world-registry'}})
        registry = json.loads(next(d for d in docs if d['metadata']['name'] == f'{name}-world-registry')['data']['world-registry.json'])
        self.assertEqual({k: v for k, v in registry.items() if k != 'workers'},
                         {'format': 'mag_world_registry_v1', 'application_id': 20231, 'client_address': '$MAG_ADVERTISE_IPV4',
                          'client_tcp_port': 10473, 'party_invite_seconds': 30})
        self.assertEqual(registry['workers'], [
            {'worker_id': 'sabotage', 'worker_host': f'{name}-gameplay-sabotage', 'worker_port': 31000, 'minimum_players': 2,
             'maximum_players': 8, 'minimum_players_per_team': [1, 1], 'draining': False},
            {'worker_id': 'acquisition', 'worker_host': f'{name}-gameplay-acq', 'worker_port': 31000, 'minimum_players': 1,
             'maximum_players': 16, 'minimum_players_per_team': [2, 2], 'draining': False},
            {'worker_id': 'domination', 'worker_host': f'{name}-gameplay-domination', 'worker_port': 31000, 'minimum_players': 2,
             'maximum_players': 8, 'minimum_players_per_team': [1, 1], 'draining': True},
        ])
        for worker in registry['workers']:
            self.assertIn(worker['worker_host'], services)
        self.assertNotIn('party_invite_seconds', json.loads(next(
            d for d in render(self.worker_values()) if d['metadata']['name'] == f'{name}-world-registry')['data']['world-registry.json']))
        def rendered_registry(**registry_values):
            return json.loads(next(d for d in render(self.worker_values(worldRegistry=registry_values))
                                   if d['metadata']['name'] == f'{name}-world-registry')['data']['world-registry.json'])
        self.assertNotIn('worker_restart_adoption', rendered_registry())
        self.assertNotIn('worker_restart_adoption', rendered_registry(workerRestartAdoption={}))
        self.assertEqual(rendered_registry(workerRestartAdoption={'enabled': False})['worker_restart_adoption'], {'enabled': False})
        self.assertEqual(rendered_registry(workerRestartAdoption={'maxAdoptions': 5, 'windowSeconds': 900})['worker_restart_adoption'],
                         {'max_adoptions': 5, 'window_seconds': 900})
        wait = init['wait-for-workers']
        self.assertEqual(wait['command'], ['python3', '/scripts/wait_for_workers.py'])
        self.assertEqual(wait['args'], ['300'] + [f'{name}-gameplay-{s}:31000' for s in ('sabotage', 'acq', 'domination')])
        self.assertEqual(list(init)[-1], 'wait-for-workers')
        self.assertNotIn('wait-for-workers', [c['name'] for c in self.medius_pod(render(self.worker_values(
            worldRegistry={'workerWaitSeconds': 0})))['spec']['initContainers']])
        scripts = next(d for d in docs if d['kind'] == 'ConfigMap' and d['metadata']['name'].endswith('-scripts'))
        self.assertIn('data_bundle.py', scripts['data'])
        self.assertIn('wait_for_workers.py', scripts['data'])
        # Workers use s3 bundles even when Medius reads an existing claim.
        claim = render({'gameplayWorkers': self.workers()[:2], 'data': {'s3': {'bucket': 'private-runtime', 'existingSecret': 'object-store'}}})
        self.assertIn('persistentVolumeClaim', next(v for v in self.medius_pod(claim)['spec']['volumes'] if v['name'] == 'data'))
        self.assertIn('data_bundle.py', next(d for d in claim if d['metadata']['name'].endswith('-scripts'))['data'])

    def test_gameplay_worker_rollout_couples_medius(self):
        """Medius never adopts a restarted worker, so anything that rolls a worker must roll Medius too."""
        def snapshot(workers=None, **extra):
            docs = render(self.worker_values(workers, **extra))
            pods = {d['metadata']['name'].replace('test-mag-recomp-server-', ''): d['spec']['template'] for d in docs if d['kind'] == 'Deployment'}
            registry = next(d for d in docs if d['metadata']['name'].endswith('-world-registry'))['data']['world-registry.json']
            return pods, registry
        base, registry = snapshot()
        annotations = base['medius']['metadata']['annotations']
        for suffix in ('sabotage', 'acq', 'domination'):
            self.assertRegex(annotations[f'checksum/gameplay-{suffix}'], '^[0-9a-f]{64}$')
        self.assertRegex(annotations['checksum/world-registry'], '^[0-9a-f]{64}$')

        def changed(workers=None, **extra):
            pods, _ = snapshot(workers, **extra)
            return {name for name in base if pods.get(name) != base[name]}
        bundle = self.workers(); bundle[0]['data']['s3']['sha256'] = 'f' * 64
        self.assertEqual(changed(bundle), {'gameplay-sabotage', 'medius'})
        args = self.workers(); args[2]['extraArgs'] = ['--collision-world', '/data/collision.json']
        self.assertEqual(changed(args), {'gameplay-domination', 'medius'})
        self.assertEqual(changed(images={'gameplay': {'tag': '9.9.9'}}),
                         {'gameplay-sabotage', 'gameplay-acq', 'gameplay-domination', 'medius'})
        self.assertEqual(changed(nodeSelector={'kubernetes.io/arch': 'arm64'}),
                         {'gameplay-sabotage', 'gameplay-domination', 'medius'})
        # Structural registry changes rebuild Medius only; workers keep running.
        minimum = self.workers(); minimum[0]['minimumPlayers'] = 4
        self.assertEqual(changed(minimum), {'medius'})
        # Drain flags reload live: only the registry file changes, no Pod restarts.
        drain = self.workers(); drain[0]['draining'] = True
        pods, drained = snapshot(drain)
        self.assertEqual(pods, base)
        self.assertNotEqual(drained, registry)
        self.assertTrue(json.loads(drained)['workers'][0]['draining'])

    def test_gameplay_worker_world_capacity_above_64(self):
        """A registry row may describe a 128- or 256-Character world; runtime.maximumPlayers stays at 64."""
        name = 'test-mag-recomp-server'
        for maximum in (64, 128, 256):
            entries = self.workers()
            entries[2] = {**entries[2], 'maximumPlayers': maximum, 'minimumPlayers': 2, 'minimumPlayersPerTeam': [1, 1]}
            docs = render(self.worker_values(entries))
            registry = json.loads(next(d for d in docs if d['metadata']['name'] == f'{name}-world-registry')['data']['world-registry.json'])
            row = next(w for w in registry['workers'] if w['worker_id'] == 'domination')
            self.assertEqual((row['maximum_players'], row['minimum_players'], row['minimum_players_per_team']), (maximum, 2, [1, 1]))
        render({'runtime': {'maximumPlayers': 65}}, ok=False)

    def test_gameplay_worker_validation(self):
        workers = self.workers()
        def invalid(changes, index=0, message=None, **extra):
            entries = self.workers()
            entries[index] = {**entries[index], **changes}
            for key in [k for k, v in changes.items() if v is None]:
                del entries[index][key]
            error = render(self.worker_values(entries, **extra), ok=False)
            if message:
                self.assertIn(message, error)
        self.assertIn('2 to 16', render(self.worker_values(workers[:1]), ok=False))
        render(self.worker_values([{**workers[0], 'id': f'w{i}'} for i in range(17)]), ok=False)
        render(self.worker_values([{**workers[0], 'id': f'w{i}'} for i in range(16)]))
        invalid({'id': 'acquisition', 'name': 'other'}, message='repeats id')
        invalid({'name': 'acq'}, message='repeats name')
        invalid({'extraArgs': ['--worker-id', 'other']}, message='chart-managed')
        invalid({'extraArgs': ['--match-profiles=/data/other.json']}, message='chart-managed')
        invalid({'extraArgs': ['--item-catalog', '/data/x.json']}, message='chart-managed')
        invalid({'minimumPlayers': 9}, message='cannot exceed')
        invalid({'name': 'a' * 32}, message='63 characters')
        for changes in ({'matchProfiles': None}, {'matchProfiles': '../profiles.json'}, {'matchProfiles': '/data/profiles.json'},
                        {'data': None}, {'data': {'s3': {'key': 'k'}}}, {'data': {'s3': {'sha256': 'a' * 64}}},
                        {'data': {'s3': {'key': 'k', 'sha256': 'bad'}}}, {'id': 'Upper'}, {'id': 'under_score'}, {'id': ''},
                        {'minimumPlayers': 0}, {'maximumPlayers': 257}, {'maximumPlayers': 0}, {'minimumPlayersPerTeam': [1]}, {'minimumPlayersPerTeam': [0, 1]},
                        {'draining': 'yes'}, {'navigation': {'s3': {'key': 'k', 'sha256': 'a' * 64}, 'mountPath': '/data'}},
                        {'navigation': {'s3': {'key': 'k', 'sha256': 'a' * 64}, 'mountPath': 'relative'}},
                        {'navigation': {'emptyDirSizeLimit': '1Gi'}}, {'unknown': True}):
            with self.subTest(changes=changes):
                invalid(changes)
        self.assertIn('20231', render(self.worker_values(runtime={'existingSecret': 'runtime-keys', 'applicationId': 20230,
                                                                  'protocolVersion': 1725, 'gameType': 2},
                                                         images={'gameplay': {'tag': '0.1.0'}}), ok=False))
        # Without a store to inherit, each worker must name its own bucket and Secret.
        error = render({'gameplayWorkers': workers[:2]}, ok=False)
        self.assertIn('s3.bucket', error)
        for invalid_registry in ({'partyInviteSeconds': 0}, {'partyInviteSeconds': 301}, {'workerWaitSeconds': -1}, {'other': 1},
                                 {'workerRestartAdoption': {'enabled': 'no'}}, {'workerRestartAdoption': {'maxAdoptions': 0}},
                                 {'workerRestartAdoption': {'windowSeconds': 86401}}, {'workerRestartAdoption': {'other': 1}}):
            render(self.worker_values(worldRegistry=invalid_registry), ok=False)

    def test_wait_for_workers_script(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('wait_for_workers', CHART / 'files' / 'wait_for_workers.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        import socket
        with socket.socket() as listener, socket.socket() as reserved:
            listener.bind(('127.0.0.1', 0)); listener.listen()
            reserved.bind(('127.0.0.1', 0))  # Bound but not listening: connections are refused.
            ready = f'127.0.0.1:{listener.getsockname()[1]}'
            closed = f'127.0.0.1:{reserved.getsockname()[1]}'
            self.assertEqual(module.wait(5, [ready]), [])
            clock = iter(range(100)).__next__
            self.assertEqual(module.wait(3, [ready, closed], clock=clock, sleep=lambda _: None), [closed])
            result = subprocess.run([os.sys.executable, str(CHART / 'files' / 'wait_for_workers.py'), '0', ready, closed],
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, 'Medius must still start without an unreachable worker')
            self.assertIn(closed, result.stderr)
            self.assertIn(f'ready: {ready}', result.stdout)
            self.assertNotEqual(subprocess.run([os.sys.executable, str(CHART / 'files' / 'wait_for_workers.py'), 'x', ready],
                                               capture_output=True, timeout=10).returncode, 0)

    def test_medius_udp_port_range(self):
        def udp(docs):
            service = next(d for d in docs if d['kind'] == 'Service' and d['metadata']['name'].endswith('-medius'))
            main = next(c for c in self.medius_pod(docs)['spec']['containers'] if c['name'] == 'medius')
            return ([p for p in service['spec']['ports'] if p['protocol'] == 'UDP'],
                    [p for p in main['ports'] if p['protocol'] == 'UDP'])
        service, container = udp(render())
        self.assertEqual(service, [{'name': 'game-udp', 'port': 10474, 'targetPort': 'game-udp', 'protocol': 'UDP'}])
        self.assertEqual(container, [{'name': 'game-udp', 'containerPort': 10474, 'protocol': 'UDP'}])
        docs = render({'network': {'udpPortRange': {'start': 10474, 'end': 10505}}})
        service, container = udp(docs)
        self.assertEqual([p['port'] for p in service], list(range(10474, 10506)))
        self.assertEqual([p['containerPort'] for p in container], list(range(10474, 10506)))
        self.assertEqual(service[1], {'name': 'game-udp-10475', 'port': 10475, 'targetPort': 10475, 'protocol': 'UDP'})
        names = [p['name'] for p in service]
        self.assertEqual(len(names), len(set(names)))
        self.assertTrue(all(len(n) <= 15 for n in names))
        pod = self.medius_pod(docs)['spec']
        declared = [(p['containerPort'], p['protocol']) for c in pod['containers'] for p in c['ports']]
        self.assertEqual(len(declared), len(set(declared)))
        for bad in ({'start': 10475, 'end': 10505}, {'start': 10474, 'end': 10473}, {'start': 10474, 'end': 10730}, {'start': 10474, 'end': '10505'}):
            with self.subTest(range=bad):
                render({'network': {'udpPortRange': bad}}, ok=False)
        # An admin API port inside the UDP range stays closed on TCP but open for UDP game traffic.
        selector = {'matchLabels': {'kubernetes.io/metadata.name': 'mag-portal'}}
        docs = render({'network': {'udpPortRange': {'start': 10474, 'end': 10505}},
                       'adminApi': {'enabled': True, 'existingSecret': 'mag-admin-api', 'allowedNamespaceSelector': selector}})
        policy = next(d for d in docs if d['kind'] == 'NetworkPolicy')
        self.assertFalse(self.policy_allows(policy, 10490, 'TCP', 'elsewhere'))
        self.assertTrue(self.policy_allows(policy, 10490, 'TCP', 'mag-portal'))
        for port in range(10474, 10506):
            self.assertTrue(self.policy_allows(policy, port, 'UDP', 'elsewhere'), port)

    def test_medius_dme_human_capacity(self):
        def env(docs):
            main = next(c for c in self.medius_pod(docs)['spec']['containers'] if c['name'] == 'medius')
            return {e['name']: e.get('value') for e in main['env'] if e['name'].startswith('MAG_DME_')}
        # The forwarded range is the admission cap: one human per port, across every world.
        self.assertEqual(env(render()), {'MAG_DME_UDP_PORT_END': '10474'})
        self.assertEqual(env(render({'network': {'udpPortRange': {'start': 10474, 'end': 10505}}})),
                         {'MAG_DME_UDP_PORT_END': '10505'})
        self.assertEqual(env(render({'network': {'udpPortRange': {'start': 10474, 'end': 10505}, 'dmeHumanCapacity': 20}})),
                         {'MAG_DME_UDP_PORT_END': '10505', 'MAG_DME_HUMAN_CAPACITY': '20'})
        for bad in (0, 33, '20'):
            with self.subTest(capacity=bad):
                render({'network': {'udpPortRange': {'start': 10474, 'end': 10505}, 'dmeHumanCapacity': bad}}, ok=False)
        # MAPS hands out no DME ports.
        for d in render():
            if d['kind'] == 'Deployment' and d['metadata']['name'].endswith('-maps'):
                for c in d['spec']['template']['spec']['containers']:
                    self.assertFalse(any(e['name'].startswith('MAG_DME_') for e in c.get('env', [])))

    def test_package(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = subprocess.run([HELM, 'package', str(CHART), '--destination', tmp], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            version = yaml.safe_load((CHART / 'Chart.yaml').read_text())['version']
            self.assertTrue((Path(tmp)/f'mag-recomp-server-{version}.tgz').exists())


if __name__ == '__main__': unittest.main()
