// Offline Release Please 17.6.0 fixture: no API requests or release creation.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {Manifest} = require('release-please/build/src/manifest');
const {validateTitle} = require('../tools/check-pr-title.cjs');
const root = path.resolve(__dirname, '..');
const configFile = 'release-please-v212-config.json';
const manifestFile = '.release-please-v212-manifest.json';
const config = JSON.parse(fs.readFileSync(path.join(root, configFile)));
const versions = JSON.parse(fs.readFileSync(path.join(root, manifestFile)));
const branch = 'codex/retail-v2.12';
const quiet = {info(){}, warn(){}, debug(){}, error(){}};
require('release-please/build/src/util/logger').setLogger(quiet);
const files = ['horizon/version.txt', 'gameplay/Cargo.toml', 'gameplay/Cargo.lock',
  'portal/Cargo.toml', 'portal/Cargo.lock', 'charts/mag-recomp-server/Chart.yaml'];
const source = {
  repository: {owner: 'example', repo: 'synthetic'},
  async getFileJson(name) {
    if (name === configFile) return config;
    if (name === manifestFile) return versions;
    throw Error('Unexpected JSON read: ' + name);
  },
  async getFileContents(name) {
    assert.ok(files.includes(name), 'Unexpected source read: ' + name);
    const text = fs.readFileSync(path.join(root, name), 'utf8');
    return {parsedContent: text, content: Buffer.from(text).toString('base64'), sha: 'fixture'};
  },
  async getFileContentsOnBranch(name) {return this.getFileContents(name);},
  async *releaseIterator() {},
  async *tagIterator() {},
  async *mergeCommitIterator() {
    yield {sha: 'a'.repeat(40), message: 'feat: synthetic 2.12 integration', files};
    yield {sha: config['bootstrap-sha'], message: 'chore: source baseline', files: []};
  }
};
(async () => {
  assert.equal(require('release-please/package.json').version, '17.6.0');
  const manifest = await Manifest.fromManifest(source, branch, configFile, manifestFile, {logger: quiet});
  const prs = await manifest.buildPullRequests();
  assert.equal(prs.length, 1);
  const pr = prs[0];
  assert.equal(validateTitle(pr.title.toString()).valid, true);
  assert.ok(pr.headRefName.includes('retail-v2.12'));
  assert.ok(!pr.updates.some(u => u.path === '.release-please-manifest.json'));
  const update = pr.updates.find(u => u.path === manifestFile);
  assert.ok(update, 'Candidate must update its separate manifest');
  const next = JSON.parse(update.updater.updateContent(JSON.stringify(versions), quiet));
  for (const directory of Object.keys(versions)) {
    assert.match(next[directory], /^\d+\.\d+\.\d+-v212(?:[.\d]*)$/, directory + ': prerelease version');
    assert.ok(require('semver').gt(next[directory], versions[directory]), directory + ': candidate must advance the baseline');
  }
  console.log('PASS 2.12 branch creates one prerelease PR and updates only its candidate manifest');
})().catch(error => {console.error(error); process.exitCode = 1;});
