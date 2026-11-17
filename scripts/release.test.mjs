import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { parseVersion, compareVersions, checkVersions, prepareVersion, releaseNotes, createReleaseTag } from './release.mjs';
import { tagVersion, checkHistory } from './ci/check-release-tag.mjs';

const packages = ['solar', 'solar-agent-core', 'solar-providers', 'solar-tools'];
function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'solar-version-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const write = (name, contents) => {
    const file = path.join(directory, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, contents);
  };
  write('Cargo.toml', '[workspace]\nmembers = ["src-tauri"]\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n\n[profile.release]\nlto = "thin"\n');
  write('package.json', '{"name":"solar-desktop","version":"0.1.0","private":true}\n');
  write('src-tauri/tauri.conf.json', '{"productName":"Solar","version":"0.1.0"}\n');
  for (const folder of ['src-tauri', 'crates/solar-agent-core', 'crates/solar-providers', 'crates/solar-tools']) {
    write(`${folder}/Cargo.toml`, '[package]\nversion.workspace = true\n');
  }
  write('Cargo.lock', 'version = 4\n\n' + [...packages.map((name) => `[[package]]\nname = "${name}"\nversion = "0.1.0"\n`), '[[package]]\nname = "external"\nversion = "0.1.0"\nsource = "registry+https://example.com"\nchecksum = "leave-me"\n'].join('\n'));
  write('CHANGELOG.md', '# Changelog\n\n## [Unreleased]\n\n## [0.1.0]\n\n- Initial Solar release.\n');
  return { directory, write };
}

test('stable release versions are bare, ordered numbers', () => {
  for (const value of ['0.1.0', '1.10.2', '12.0.0']) assert.doesNotThrow(() => parseVersion(value));
  for (const value of ['v0.1.0', '01.1.0', '1.0', '1.0.0-beta.1', '1.0.0+build', ' 1.0.0', '1.0.0\n', '-1.0.0']) assert.throws(() => parseVersion(value));
  assert.equal(compareVersions('1.10.0', '1.9.9'), 1);
});

test('prepare updates every local version without modifying dependencies', (t) => {
  const { directory } = fixture(t);
  assert.equal(prepareVersion(directory, 'minor'), '0.2.0');
  assert.equal(checkVersions(directory), '0.2.0');
  assert.match(fs.readFileSync(path.join(directory, 'Cargo.lock'), 'utf8'), /name = "external"\nversion = "0.1.0"\nsource = "registry\+https:\/\/example.com"\nchecksum = "leave-me"/);
  assert.match(fs.readFileSync(path.join(directory, 'Cargo.toml'), 'utf8'), /\[profile.release\]\nlto = "thin"/);
  assert.throws(() => prepareVersion(directory, '0.1.0'), /backwards/);
});

test('version drift and missing release notes fail early', (t) => {
  const { directory, write } = fixture(t);
  write('src-tauri/tauri.conf.json', '{"version":"0.2.0"}');
  assert.throws(() => checkVersions(directory), /expected 0.1.0/);
  assert.throws(() => releaseNotes(directory, '0.2.0'), /CHANGELOG/);
});

test('only a non-deleted version tag push permits signing', () => {
  assert.equal(tagVersion('push', 'refs/tags/0.1.0', 'abc123'), '0.1.0');
  for (const args of [
    ['workflow_dispatch', 'refs/tags/0.1.0', 'abc123'],
    ['push', 'refs/heads/main', 'abc123'],
    ['push', 'refs/tags/v0.1.0', 'abc123'],
    ['push', 'refs/tags/0.1.0', '0'.repeat(40)],
  ]) assert.throws(() => tagVersion(...args));
});

test('moved version tags are rejected; original failed runs can be retried', () => {
  assert.doesNotThrow(() => checkHistory([{ event: 'push', path: 'macos-release.yml@refs/tags/0.1.0', head_branch: '', head_sha: 'same' }], '0.1.0', 'same'));
  assert.throws(() => checkHistory([{ event: 'push', path: 'macos-release.yml@refs/tags/0.1.0', head_branch: '', head_sha: 'old' }], '0.1.0', 'new'), /different commit/);
  assert.doesNotThrow(() => checkHistory([{ event: 'push', path: 'macos-release.yml@refs/heads/0.1.0', head_branch: '0.1.0', head_sha: 'old' }], '0.1.0', 'new'));
});

test('release tags require committed, published main and cannot be overwritten', (t) => {
  const { directory, write } = fixture(t);
  const remote = `${directory}-remote`;
  t.after(() => fs.rmSync(remote, { recursive: true, force: true }));
  const git = (...args) => execFileSync('git', args, { cwd: directory, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
  git('init', '-b', 'main');
  git('config', 'user.name', 'Solar Test');
  git('config', 'user.email', 'solar-test@example.invalid');
  git('config', 'commit.gpgsign', 'false');
  git('config', 'tag.gpgsign', 'false');
  git('add', '.');
  git('commit', '-m', 'Initial fixture');
  git('init', '--bare', remote);
  git('remote', 'add', 'origin', remote);
  git('push', '-u', 'origin', 'main');
  write('pending.txt', 'not committed');
  assert.throws(() => createReleaseTag(directory, '0.1.0'), /Commit all changes/);
  fs.unlinkSync(path.join(directory, 'pending.txt'));
  assert.equal(createReleaseTag(directory, '0.1.0', true), '0.1.0');
  assert.equal(git('cat-file', '-t', '0.1.0'), 'tag');
  assert.match(git('ls-remote', '--tags', 'origin'), /refs\/tags\/0.1.0/);
  assert.throws(() => createReleaseTag(directory, '0.1.0'), /never overwritten/);
});
