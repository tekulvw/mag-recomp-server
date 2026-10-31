const fs = require('node:fs');
const {parseConventionalCommits} = require('release-please/build/src/commit');

const types = new Set(['feat','fix','perf','refactor','docs','test','build','ci','chore','style','revert']);
const quiet = {debug(){},info(){},warn(){},error(){}};
const guidance = 'Use <type>(optional scope)!: description; scope and ! are optional. ' +
  'Types: '+[...types].join(', ')+'. Example: fix(medius): reject invalid packets.';

function validateTitle(title) {
  if (typeof title !== 'string' || !title || title !== title.trim() || /[\r\n\u2028\u2029]/u.test(title)) {
    return {valid:false};
  }
  const commits = parseConventionalCommits([{sha:'synthetic-title',message:title,files:[]}],quiet);
  return {valid:commits.length === 1 && types.has(commits[0].type) && Boolean(commits[0].bareMessage.trim())};
}

function main() {
  let title;
  try {
    title = process.argv[2] ?? JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH,'utf8')).pull_request?.title;
  } catch {
    console.error('::error::Unable to read a pull request title from the event JSON.');
    return 1;
  }
  if (!validateTitle(title).valid) {
    // Do not echo untrusted title text into workflow commands or shell scripts.
    console.error('::error::Invalid pull request title. '+guidance);
    return 1;
  }
  console.log('PR title is a valid Conventional Commit header.');
  return 0;
}

module.exports = {validateTitle};
if (require.main === module) process.exitCode = main();
