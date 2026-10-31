// Offline fixtures use the same release-please version bundled by the pinned action.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {Manifest} = require('release-please/build/src/manifest');
const YAML = require('yaml');
assert.equal(require('release-please/package.json').version, '17.6.0');
const root = path.resolve(__dirname, '..');
const config = JSON.parse(fs.readFileSync(path.join(root, 'release-please-config.json')));
const current = JSON.parse(fs.readFileSync(path.join(root, '.release-please-manifest.json')));
const chartPath = 'charts/mag-recomp-server';
const paths = ['horizon', 'gameplay', chartPath];
const seed = {horizon:'0.1.0',gameplay:'0.1.0',[chartPath]:'0.1.0'};
const quiet = {info(){}, warn(){}, debug(){}, error(){}};
require('release-please/build/src/util/logger').setLogger(quiet);
const chart = 'apiVersion: v2\nname: mag-recomp-server\nversion: 0.1.0\nappVersion: "0.1.0"\n';
const cargo = '[package]\nname = "mag-gameplay"\nversion = "0.1.0"\nedition = "2024"\n';
const lock = 'version = 4\n\n[[package]]\nname = "mag-gameplay"\nversion = "0.1.0"\n\n[[package]]\nname = "example-dependency"\nversion = "1.2.3"\n';

async function candidates(changes, versions=seed, allReleased=false) {
  const boundaries = {horizon:'b'.repeat(40), gameplay:'c'.repeat(40), [chartPath]:'d'.repeat(40)};
  const files = {'horizon/version.txt':'0.1.0\n', 'gameplay/Cargo.toml':cargo,
    'gameplay/Cargo.lock':lock, [`${chartPath}/Chart.yaml`]:chart};
  const source = {
    repository:{owner:'example', repo:'synthetic'},
    async getFileJson(name) {
      if (name === 'release-please-config.json') return config;
      if (name === '.release-please-manifest.json') return versions;
      throw Error(`Unexpected JSON request: ${name}`);
    },
    async getFileContents(name) {
      assert.ok(name in files, `Unexpected file request: ${name}`);
      return {parsedContent:files[name], content:Buffer.from(files[name]).toString('base64'), sha:'synthetic'};
    },
    async getFileContentsOnBranch(name) { return this.getFileContents(name); },
    async *releaseIterator() {
      for (const [component, directory] of [['chart',chartPath], ...(allReleased ? [['medius','horizon'],['gameplay','gameplay']] : [])]) {
        yield {tagName:`${component}-v${versions[directory]}`, sha:allReleased ? boundaries[directory] : config['bootstrap-sha'], notes:''};
      }
    },
    async *tagIterator(){},
    async *mergeCommitIterator() {
      for (const [index, change] of changes.entries()) {
        yield {sha:String(index + 1).repeat(40).slice(0,40), ...change};
      }
      if (allReleased) {
        for (const directory of paths) yield {sha:boundaries[directory], message:'chore: release', files:[`${directory}/CHANGELOG.md`]};
      }
      yield {sha:config['bootstrap-sha'], message:'chore: release chart 0.1.0', files:[`${chartPath}/Chart.yaml`]};
      // An existing feature predates the migration. It must never bump new packages.
      yield {sha:'e'.repeat(40), message:'feat: historical published implementation', files:['horizon/Server.Medius/Program.cs','gameplay/src/main.rs']};
    }
  };
  const manifest = await Manifest.fromManifest(source, 'main', undefined, undefined, {logger:quiet});
  const prs = await manifest.buildPullRequests();
  source.pullRequestIterator = async function*() {
    for (const pr of prs) yield {number:1, title:pr.title.toString(), body:pr.body.toString(),
      headBranchName:pr.headRefName, baseBranchName:'main', labels:['autorelease: pending'],
      sha:'f'.repeat(40), files:pr.updates.map(u=>u.path)};
  };
  return {prs, releases:await manifest.buildReleases()};
}

async function check(name, changes, bumps, versions=seed, allReleased=false) {
  const {prs,releases} = await candidates(changes, versions, allReleased);
  assert.equal(prs.length, Object.keys(bumps).length ? 1 : 0, `${name}: one combined PR`);
  for (const pr of prs) assert.equal(pr.headRefName,'release-please--branches--main');
  const components = {horizon:'medius',gameplay:'gameplay',[chartPath]:'chart'};
  assert.deepEqual(releases.map(r=>r.tag.toString()).sort(),Object.entries(bumps).map(([p,v])=>`${components[p]}-v${v}`).sort(),`${name}: component release tags`);
  assert.deepEqual(releases.map(r=>r.path).sort(),Object.keys(bumps).sort(),`${name}: component output paths`);
  assert.ok(releases.every(r=>r.sha === 'f'.repeat(40)),`${name}: common release commit`);
  if (!prs.length) { console.log(`PASS ${name}`); return; }
  const updates = prs[0].updates;
  const versionPaths = ['horizon/version.txt','gameplay/Cargo.toml',`${chartPath}/Chart.yaml`];
  const expectedPaths = Object.keys(bumps).map(p => versionPaths[paths.indexOf(p)]).sort();
  assert.deepEqual(updates.map(u=>u.path).filter(p=>versionPaths.includes(p)).sort(), expectedPaths, `${name}: affected packages`);
  let manifestText = JSON.stringify(versions);
  for (const update of updates.filter(u=>u.path === '.release-please-manifest.json')) {
    manifestText = update.updater.updateContent(manifestText, quiet);
  }
  assert.deepEqual(JSON.parse(manifestText), {...versions,...bumps}, `${name}: complete shared manifest`);
  assert.ok(!updates.some(u=>u.path === 'version.txt' || u.path === `${chartPath}/values.yaml`));
  if (bumps.horizon) {
    const text = updates.find(u=>u.path==='horizon/version.txt').updater.updateContent('0.1.0\n',quiet);
    assert.equal(text.trim(),bumps.horizon);
  }
  if (bumps.gameplay) {
    const text = updates.find(u=>u.path==='gameplay/Cargo.toml').updater.updateContent(cargo,quiet);
    assert.match(text,new RegExp(`version = "${bumps.gameplay.replaceAll('.','\\.')}"`));
    const updatedLock = updates.find(u=>u.path==='gameplay/Cargo.lock').updater.updateContent(lock,quiet);
    assert.match(updatedLock,new RegExp(`name = "mag-gameplay"\\nversion = "${bumps.gameplay.replaceAll('.','\\.')}"`));
    assert.match(updatedLock,/name = "example-dependency"\nversion = "1\.2\.3"/);
  }
  if (bumps[chartPath]) {
    const text = updates.find(u=>u.path===`${chartPath}/Chart.yaml`).updater.updateContent(chart,quiet);
    assert.equal(YAML.parse(text).version,bumps[chartPath]);
    assert.equal(YAML.parse(text).appVersion,'0.1.0');
  }
  console.log(`PASS ${name}`);
}

function publicationContract() {
  const workflow = YAML.parse(fs.readFileSync(path.join(root,'.github/workflows/release-please.yml'),'utf8'));
  const jobs = workflow.jobs;
  for (const [component,directory,suffix] of [['medius','horizon','medius'],['gameplay','gameplay','game']]) {
    for (const [output,source] of [['release_created','release_created'],['tag_name','tag_name'],['version','version']]) {
      assert.equal(jobs['release-please'].outputs[`${component}_${output}`], `\${{ steps.release.outputs['${directory}--${source}'] }}`);
    }
    const publisher = jobs[`publish-${component}`];
    assert.equal(publisher.if, `needs.release-please.outputs.${component}_release_created == 'true'`);
    assert.ok(publisher.needs.includes('application-release-checks'));
    assert.equal(publisher.uses,'./.github/workflows/publish-image.yml');
    assert.equal(publisher.with.context,directory);
    assert.equal(publisher.with.image_suffix,suffix);
    assert.equal(publisher.with.ref,`\${{ needs.release-please.outputs.${component}_tag_name }}`);
    assert.equal(publisher.with.version,`\${{ needs.release-please.outputs.${component}_version }}`);
  }
  const appChecks = jobs['application-release-checks'];
  assert.equal(appChecks.if,"needs.release-please.outputs.medius_release_created == 'true' || needs.release-please.outputs.gameplay_release_created == 'true'");
  assert.equal(jobs['publish-chart'].if,"needs.release-please.outputs.chart_release_created == 'true'");
  const publisher = YAML.parse(fs.readFileSync(path.join(root,'.github/workflows/publish-image.yml'),'utf8'));
  const metadata = publisher.jobs.publish.steps.find(s=>s.id==='meta');
  assert.ok(metadata.with.tags.includes('value=${{ inputs.version }}'));
  assert.ok(!metadata.with.tags.includes('inputs.ref'));
  assert.equal(publisher.jobs.publish.steps.find(s=>s.name==='Check out the release tag').with.ref,'${{ inputs.ref }}');
  console.log('PASS independent publication gates and plain SemVer image tags');
}

(async()=>{
  assert.deepEqual(Object.keys(config.packages).sort(), [...paths].sort());
  assert.equal(config['separate-pull-requests'],false);
  assert.match(config['bootstrap-sha'],/^[0-9a-f]{40}$/);
  assert.deepEqual(Object.keys(current).sort(),[...paths].sort());
  assert.equal(fs.readFileSync(path.join(root,'horizon/version.txt'),'utf8').trim(),current.horizon);
  assert.match(fs.readFileSync(path.join(root,'gameplay/Cargo.toml'),'utf8'),new RegExp(`^version = "${current.gameplay.replaceAll('.','\\.')}"$`,'m'));
  assert.equal(YAML.parse(fs.readFileSync(path.join(root,chartPath,'Chart.yaml'),'utf8')).version,current[chartPath]);
  await check('migration does not replay published history',[],{});
  await check('migration chore creates no releases',[{message:'chore: version server components independently',files:['release-please-config.json','.release-please-manifest.json','horizon/version.txt']}],{});
  await check('root documentation does not release packages',[{message:'fix: clarify deployment',files:['README.md']}],{});
  await check('sibling directories do not release packages',[{message:'fix: synthetic sibling',files:['horizon-other/example.cs','gameplay-other/example.rs',`${chartPath}-other/values.yaml`]}],{});
  await check('Medius-only patch',[{message:'fix: synthetic Medius fix',files:['horizon/Server.Medius/Program.cs']}],{horizon:'0.1.1'});
  await check('gameplay-only patch',[{message:'fix: synthetic gameplay fix',files:['gameplay/src/main.rs']}],{gameplay:'0.1.1'});
  await check('chart-only patch',[{message:'fix: synthetic chart fix',files:[`${chartPath}/values.yaml`]}],{[chartPath]:'0.1.1'});
  await check('mixed independent bumps in one PR',[
    {message:'fix: synthetic Medius fix',files:['horizon/Server.Medius/Program.cs']},
    {message:'feat: synthetic gameplay feature',files:['gameplay/src/main.rs']},
    {message:'fix: synthetic chart fix',files:[`${chartPath}/values.yaml`]}
  ],{horizon:'0.1.1',gameplay:'0.2.0',[chartPath]:'0.1.1'});
  const divergent={horizon:'0.4.0',gameplay:'0.7.0',[chartPath]:'0.2.0'};
  await check('later releases keep independent versions',[
    {message:'fix: synthetic pair fix',files:['horizon/Server.Medius/Program.cs','gameplay/src/main.rs']}
  ],{horizon:'0.4.1',gameplay:'0.7.1'},divergent,true);
  publicationContract();
})().catch(error=>{console.error(error);process.exitCode=1;});
