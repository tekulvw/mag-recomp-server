const assert = require('node:assert/strict');
const {test} = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawnSync} = require('node:child_process');
const YAML = require('yaml');
const {validateTitle} = require('../tools/check-pr-title.cjs');

for (const title of [
  'feat: add a feature', 'fix(medius): reject an invalid packet',
  'fix(gameplay)!: change the bridge contract', 'feat!: change an interface',
  'docs: clarify deployment', 'test: add synthetic coverage',
  'build(deps): update dependencies', 'ci: validate PR titles',
  'perf: reduce allocations', 'refactor: simplify parsing',
  'style: format source', 'revert: remove a feature',
  'chore(main): release', 'chore(main): release 0.1.0',
  'chore(main): release chart 0.1.0', 'fix: Allow Unicode descriptions — example'
]) {
  test(`accept ${JSON.stringify(title)}`,()=>assert.equal(validateTitle(title).valid,true));
}
for (const title of [
  '', ' ', 'Add a feature', 'Merge pull request #2 from example/feature',
  'feat:', 'feat: ', 'feat(): empty scope', 'feat : misplaced space',
  'fix(api) missing colon', 'Fix: uppercase type', 'custom: unknown type',
  'fix: first line\nfeat: second line', 'fix: hidden\rline',
  ' fix: leading whitespace', 'fix: trailing whitespace '
]) {
  test(`reject ${JSON.stringify(title)}`,()=>assert.equal(validateTitle(title).valid,false));
}
test('reject missing or non-string event title',()=>{
  for (const title of [undefined,null,42,{},[]]) assert.equal(validateTitle(title).valid,false);
});

function runEvent(event) {
  const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'synthetic-pr-title-'));
  try {
    const eventPath=path.join(temporary,'event.json');
    fs.writeFileSync(eventPath,typeof event === 'string' ? event : JSON.stringify(event));
    return spawnSync(process.execPath,[path.resolve(__dirname,'../tools/check-pr-title.cjs')],
      {env:{...process.env,GITHUB_EVENT_PATH:eventPath},encoding:'utf8'});
  } finally { fs.rmSync(temporary,{recursive:true,force:true}); }
}
test('CLI reads edited title from JSON event and never executes its contents',()=>{
  const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'synthetic-title-command-'));
  try {
    const marker=path.join(temporary,'must-not-exist');
    const title='fix: literal $(touch '+marker+') `touch '+marker+'`';
    const result=runEvent({action:'edited',pull_request:{title}});
    assert.equal(result.status,0,result.stderr);
    assert.equal(fs.existsSync(marker),false);
  } finally { fs.rmSync(temporary,{recursive:true,force:true}); }
});
test('CLI fails malformed titles with static guidance',()=>{
  const result=runEvent({pull_request:{title:'invalid title'}});
  assert.equal(result.status,1);
  assert.match(result.stderr,/<type>\(optional scope\)!: description/);
});
test('CLI fails closed on missing title or malformed event JSON',()=>{
  for (const event of [{}, {pull_request:{}}, '{broken']) assert.equal(runEvent(event).status,1);
});
test('workflow reruns title edits and reads JSON without shell title interpolation',()=>{
  const source=fs.readFileSync(path.resolve(__dirname,'../.github/workflows/pr-title.yml'),'utf8');
  const workflow=YAML.parse(source);
  for (const event of ['opened','reopened','synchronize','edited','ready_for_review']) {
    assert.ok(workflow.on.pull_request.types.includes(event));
  }
  assert.deepEqual(workflow.permissions,{contents:'read'});
  assert.equal(workflow.jobs['pr-title'].steps[0].with['persist-credentials'],false);
  assert.ok(!source.includes('github.event.pull_request.title'));
  assert.match(workflow.jobs['pr-title'].steps.at(-1).run,/node tools\/check-pr-title\.cjs$/);
});
