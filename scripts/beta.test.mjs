import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { betaContext, previousBeta, betaNeedsBuild, betaNotes, checkBetaBuild } from './ci/check-beta-build.mjs';
import { rejectStaleVersion } from './ci/check-release-tag.mjs';

const SHA = 'a'.repeat(40);
const OLD = 'b'.repeat(40);
const currentBeta = (commit = SHA) => ({
  tag_name: 'beta', prerelease: true, draft: false, target_commitish: commit,
  body: `Notes\n\n<!-- solar-beta:v1 commit=${commit} -->`,
  assets: [{ name: `Solar_beta_${commit}_aarch64.dmg`, size: 1234 }, { name: 'SHA256SUMS', size: 100 }],
});

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'solar-beta-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const write = (name, text) => {
    const file = path.join(directory, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text);
  };
  write('Cargo.toml', '[workspace.package]\nversion = "0.1.3"\nedition = "2024"\n');
  write('package.json', '{"version":"0.1.3"}\n');
  write('src-tauri/tauri.conf.json', '{"version":"0.1.3"}\n');
  for (const folder of ['src-tauri', 'crates/solar-agent-core', 'crates/solar-providers', 'crates/solar-tools']) {
    write(`${folder}/Cargo.toml`, '[package]\nversion.workspace = true\n');
  }
  write('Cargo.lock', ['solar', 'solar-agent-core', 'solar-providers', 'solar-tools'].map(name =>
    `[[package]]\nname = "${name}"\nversion = "0.1.3"\n`).join('\n'));
  write('output', '');
  write('environment', '');
  const env = {
    GITHUB_EVENT_NAME: 'schedule', GITHUB_REF: 'refs/heads/main', GITHUB_SHA: OLD,
    GITEA_REPOSITORY: 'internal/solar-chat', GITHUB_REPOSITORY: 'internal/solar-chat',
    GITEA_SERVER_URL: 'https://git.example.invalid', GITEA_TOKEN: 'dummy-token',
    RUNNER_TEMP: directory, SOLAR_JOB_ID: '123-1',
    GITHUB_OUTPUT: path.join(directory, 'output'), GITHUB_ENV: path.join(directory, 'environment'),
  };
  const commands = [];
  const git = (...args) => {
    commands.push(args);
    if (args[0] === 'rev-parse') return SHA;
    if (args[0] === 'status' || args[0] === 'merge-base') return '';
    if (args[0] === 'log') return 'abc1234 Improve Sprites\ndef1234 Fix popup';
    throw new Error(`Unexpected Git command: ${args[0]}`);
  };
  return { directory, env, git, commands };
}

test('beta signing is restricted to scheduled or manual main builds on the same repository', t => {
  const { env } = fixture(t);
  assert.equal(betaContext(env).repository, 'internal/solar-chat');
  for (const patch of [
    { GITHUB_EVENT_NAME: 'push' }, { GITHUB_REF: 'refs/tags/beta' },
    { GITHUB_REF: 'refs/heads/feature' }, { GITHUB_REPOSITORY: 'other/project' },
    { GITEA_REPOSITORY: '../project' }, { GITEA_SERVER_URL: 'http://git.example.invalid' },
    { GITEA_SERVER_URL: 'https://token@git.example.invalid' },
    { GITEA_SERVER_URL: 'https://git.example.invalid/path' },
    { GITEA_SERVER_URL: 'https://git.example.invalid?token=hidden' },
    { GITEA_TOKEN: 'bad token' },
  ]) assert.throws(() => betaContext({ ...env, ...patch }));
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

test('scheduled context SHA may be stale but checkout must match live main', async t => {
  const f = fixture(t);
  const api = async suffix => suffix === '/branches/main'
    ? { commit: { id: SHA } } : suffix === '/releases/tags/beta' ? currentBeta(OLD) : null;
  const result = await checkBetaBuild({ ...f, api });
  assert.deepEqual(result, { build: true, sha: SHA, version: '0.1.3', previous: OLD });
  const output = fs.readFileSync(f.env.GITHUB_OUTPUT, 'utf8');
  const environment = fs.readFileSync(f.env.GITHUB_ENV, 'utf8');
  assert.match(output, new RegExp(`build=true\nversion=0.1.3\nsha=${SHA}`));
  assert.match(environment, new RegExp(`SOLAR_RELEASE_SHA=${SHA}`));
  assert.match(environment, new RegExp(`SOLAR_PREVIOUS_BETA_SHA=${OLD}`));
  assert.ok(f.commands.some(command => command.includes(`${OLD}..${SHA}`)));
  assert.match(fs.readFileSync(path.join(f.directory, 'solar-notes-123-1/release-notes.md'), 'utf8'), /Improve Sprites/);
});

test('dirty, stale and divergent main checkouts are rejected before signing', async t => {
  for (const mode of ['dirty', 'stale', 'divergent']) {
    const f = fixture(t);
    const git = (...args) => mode === 'dirty' && args[0] === 'status' ? ' M package.json'
      : mode === 'divergent' && args[1] === 'refs/remotes/origin/main' ? OLD : f.git(...args);
    const api = async () => ({ commit: { id: OLD } });
    await assert.rejects(checkBetaBuild({ ...f, git, api }), /clean checkout|advanced/);
    assert.equal(fs.readFileSync(f.env.GITHUB_OUTPUT, 'utf8'), '');
    assert.equal(fs.readFileSync(f.env.GITHUB_ENV, 'utf8'), '');
  }
});

test('unchanged schedule emits skip without creating notes; manual run still rebuilds', async t => {
  for (const event of ['schedule', 'workflow_dispatch']) {
    const f = fixture(t);
    f.env.GITHUB_EVENT_NAME = event;
    const api = async suffix => suffix === '/branches/main' ? { commit: { id: SHA } }
      : suffix === '/releases/tags/beta' ? currentBeta() : { commit: { sha: SHA } };
    const result = await checkBetaBuild({ ...f, api });
    assert.equal(result.build, event === 'workflow_dispatch');
    if (event === 'schedule') {
      assert.equal(fs.readFileSync(f.env.GITHUB_OUTPUT, 'utf8'), 'build=false\n');
      assert.equal(fs.readFileSync(f.env.GITHUB_ENV, 'utf8'), '');
      assert.equal(fs.existsSync(path.join(f.directory, 'solar-notes-123-1')), false);
    } else {
      assert.match(fs.readFileSync(path.join(f.directory, 'solar-notes-123-1/release-notes.md'), 'utf8'), /Manually rebuilt main with no source changes/);
    }
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
    server: new URL('https://git.example.invalid'), repository: 'internal/solar-chat' });
  assert.ok(command.includes(SHA));
  assert.ok(!command.includes(`${OLD}..${SHA}`));
  assert.match(notes, /Main history changed/);
  assert.match(notes, /latest 200 commits/);
  assert.match(notes, new RegExp(`compare/${OLD}\.\.\.${SHA}`));
  assert.doesNotMatch(notes, /\u0001/);
});

test('retired or lower stable releases cannot replace a newer retained source tag or download', () => {
  assert.doesNotThrow(() => rejectStaleVersion('0.1.4', ['0.1.3', '0.1.4', 'beta'], [{ tag_name: '0.1.3' }]));
  assert.throws(() => rejectStaleVersion('0.1.2', ['0.1.2', '0.1.3'], []), /newer than existing tag/);
  assert.throws(() => rejectStaleVersion('0.1.2', [], [{ tag_name: '0.1.3' }]), /newer than published release/);
  assert.throws(() => rejectStaleVersion('0.1.3', [], [{ tag_name: '0.1.3' }]), /newer than published release/);
  assert.doesNotThrow(() => rejectStaleVersion('0.2.0', ['0.1.99'], [{ tag_name: 'beta', prerelease: true }, { tag_name: '0.3.0', draft: true }]));
});
