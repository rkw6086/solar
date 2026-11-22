import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { root } from './release.mjs';
import { collectPackages, platforms, validateSource } from './ci/desktop-build.mjs';

const sha = 'a'.repeat(40);
const env = { GITHUB_ACTIONS: 'true', GITHUB_SERVER_URL: 'https://github.com',
  GITHUB_REPOSITORY: 'rkw6086/solar', GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/heads/main',
  GITHUB_SHA: sha, SOLAR_RELEASE_SHA: sha, SOLAR_JOB_ID: '12-1', RUNNER_TEMP: os.tmpdir() };
const source = (patch = {}, dirty = '') => validateSource({ env: { ...env, ...patch },
  directory: root, platform: 'linux', arch: 'x64',
  git: (command, ref) => command === 'rev-parse' ? (ref === 'HEAD' || ref === `${sha}^{commit}` ? sha : 'b'.repeat(40)) : dirty });

test('preview packaging binds native platform to the clean triggering source', () => {
  assert.equal(source().target, 'x86_64-unknown-linux-gnu');
  assert.throws(() => source({ GITHUB_SHA: 'b'.repeat(40) }), /triggering source/);
  assert.throws(() => source({}, ' M Cargo.toml'), /triggering source/);
  assert.throws(() => source({ GITHUB_REF: 'refs/tags/0.1.7' }), /main push/);
  assert.throws(() => source({ GITHUB_REPOSITORY: 'another/repository' }), /Solar GitHub/);
  assert.throws(() => source({ RUNNER_TEMP: 'relative' }), /job-owned/);
  assert.equal(source({ GITHUB_EVENT_NAME: 'pull_request', GITHUB_BASE_REF: 'main', GITHUB_REF: 'refs/pull/1/merge' }).name, 'linux');
});

test('release and beta builds require their validated source identities', () => {
  assert.equal(source({ SOLAR_BUILD_CHANNEL: 'release', SOLAR_RELEASE_VERSION: '0.1.7', GITHUB_REF: 'refs/tags/0.1.7' }).channel, 'release');
  assert.throws(() => source({ SOLAR_BUILD_CHANNEL: 'release', SOLAR_RELEASE_VERSION: '0.1.7' }), /exact version tag/);
  assert.throws(() => source({ SOLAR_BUILD_CHANNEL: 'release', SOLAR_RELEASE_VERSION: '0.1.6', GITHUB_REF: 'refs/tags/0.1.7' }), /differs/);
  assert.equal(source({ SOLAR_BUILD_CHANNEL: 'beta', SOLAR_RELEASE_VERSION: '0.1.7', GITHUB_EVENT_NAME: 'schedule', SOLAR_BETA_ID: sha }).channel, 'beta');
  assert.throws(() => source({ SOLAR_BUILD_CHANNEL: 'beta', GITHUB_EVENT_NAME: 'schedule' }), /validated main/);
});

function fixture(t, platform = 'linux') {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'solar-desktop-package-test-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const config = { ...platforms[platform], targetDir: path.join(directory, 'target'),
    artifactsDir: path.join(directory, 'artifacts'), version: '0.1.7', sha };
  const files = config.packages.map(([folder, extension]) => {
    const dir = path.join(config.targetDir, config.target, 'release', 'bundle', folder);
    fs.mkdirSync(dir, { recursive: true });
    const file = path.join(dir, `original${extension}`);
    fs.writeFileSync(file, `synthetic ${folder} fixture`);
    return file;
  });
  return { config, files };
}

test('both platform package sets include exact checksums and preview provenance', t => {
  for (const platform of ['linux', 'win32']) {
    const { config } = fixture(t, platform);
    const files = collectPackages({ config });
    const checksums = fs.readFileSync(path.join(config.artifactsDir, 'SHA256SUMS'), 'utf8');
    for (const file of files) {
      assert.ok(checksums.includes(`${crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')}  ${path.basename(file)}\n`));
    }
    const info = JSON.parse(fs.readFileSync(path.join(config.artifactsDir, 'BUILD-INFO.json')));
    assert.equal(info.source, sha);
    assert.equal(info.signed, false);
    assert.equal(info.channel, 'preview');
    assert.throws(() => collectPackages({ config }), /must be new/);
  }
});

test('partial, ambiguous, or empty outputs cannot become downloadable artifacts', t => {
  for (const kind of ['missing', 'duplicate', 'empty']) {
    const { config, files } = fixture(t);
    if (kind === 'missing') fs.unlinkSync(files[0]);
    if (kind === 'duplicate') fs.copyFileSync(files[0], path.join(path.dirname(files[0]), 'duplicate.deb'));
    if (kind === 'empty') fs.writeFileSync(files[0], '');
    assert.throws(() => collectPackages({ config }), /Expected one|Invalid/);
    assert.equal(fs.existsSync(config.artifactsDir), false);
  }
});

test('symlinked package outputs are rejected', { skip: process.platform === 'win32' }, t => {
  const { config, files } = fixture(t);
  fs.unlinkSync(files[0]);
  fs.symlinkSync(files[1], files[0]);
  assert.throws(() => collectPackages({ config }), /Invalid/);
});
