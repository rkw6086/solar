import test from 'node:test';
import assert from 'node:assert/strict';
import { previousBeta, betaNeedsBuild, betaNotes } from './ci/release-channels.mjs';

const SHA = 'a'.repeat(40);
const OLD = 'b'.repeat(40);
const currentBeta = (commit = SHA) => ({
  tag_name: 'beta', prerelease: true, draft: false, target_commitish: commit,
  body: `Notes\n\n<!-- solar-beta:v1 commit=${commit} -->`,
  assets: [{ name: `Solar_beta_${commit}_aarch64.dmg`, size: 1234 }, { name: 'SHA256SUMS', size: 100 }],
});

test('only a complete owned same-source scheduled beta is skipped', () => {
  const beta = currentBeta();
  const tag = { commit: { sha: SHA } };
  assert.equal(betaNeedsBuild('schedule', beta, tag, SHA), false);
  assert.equal(betaNeedsBuild('workflow_dispatch', beta, tag, SHA), true);
  assert.equal(betaNeedsBuild('schedule', beta, null, SHA), true);
  assert.equal(betaNeedsBuild('schedule', beta, { commit: { sha: OLD } }, SHA), true);
  assert.equal(betaNeedsBuild('schedule', currentBeta(OLD), tag, SHA), true);
  assert.equal(betaNeedsBuild('schedule', { ...beta, assets: beta.assets.slice(0, 1) }, tag, SHA), true);
  assert.equal(betaNeedsBuild('schedule', { ...beta, assets: beta.assets.map(a => ({ ...a, size: 0 })) }, tag, SHA), true);
  assert.equal(betaNeedsBuild('schedule', { ...beta, target_commitish: OLD }, tag, SHA), true);
  for (const patch of [{ draft: true }, { prerelease: false }, { tag_name: 'different' }, { body: 'Manual beta' },
    { body: `${beta.body}\n${beta.body}` }]) {
    assert.throws(() => previousBeta({ ...beta, ...patch }), /left unchanged/);
  }
});

test('notes use the published baseline, sanitize controls and bound long histories', () => {
  let command;
  const git = (...args) => {
    if (args[0] === 'merge-base') throw new Error('history rewritten');
    command = args;
    return Array.from({ length: 201 }, (_, i) => `${i} title\u0001`).join('\n');
  };
  const notes = betaNotes({ sha: SHA, previous: OLD, eventName: 'schedule', git,
    server: new URL('https://github.com'), repository: 'rkw6086/solar' });
  assert.ok(command.includes(SHA));
  assert.ok(!command.includes(`${OLD}..${SHA}`));
  assert.match(notes, /Main history changed/);
  assert.match(notes, /latest 200 commits/);
  assert.match(notes, new RegExp(`compare/${OLD}\.\.\.${SHA}`));
  assert.doesNotMatch(notes, /\u0001/);
});
