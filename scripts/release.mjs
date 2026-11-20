#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const localPackages = ['solar', 'solar-agent-core', 'solar-providers', 'solar-tools'];
const stableVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

export function parseVersion(value) {
  if (typeof value !== 'string' || !stableVersion.test(value)) {
    throw new Error('Use a bare stable version such as 0.1.0 (no v prefix or prerelease suffix).');
  }
  return value.split('.').map(BigInt);
}

export function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return 0;
}

function workspaceVersion(text) {
  const match = text.match(/^\[workspace\.package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m);
  const version = match?.[1].match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
  if (!version) throw new Error('Cargo.toml is missing workspace.package.version.');
  return version;
}

function lockVersions(text) {
  const found = new Map();
  for (const block of text.split(/(?=^\[\[package\]\]$)/m)) {
    const name = block.match(/^name = "([^"]+)"$/m)?.[1];
    if (!localPackages.includes(name) || /^source = /m.test(block)) continue;
    if (found.has(name)) throw new Error(`Duplicate local Cargo.lock package: ${name}`);
    found.set(name, block.match(/^version = "([^"]+)"$/m)?.[1]);
  }
  for (const name of localPackages) {
    if (!found.get(name)) throw new Error(`Cargo.lock is missing local package ${name}.`);
  }
  return found;
}

export function versionState(directory = root) {
  const read = (file) => fs.readFileSync(path.join(directory, file), 'utf8');
  return new Map([
    ['Cargo.toml', workspaceVersion(read('Cargo.toml'))],
    ['package.json', JSON.parse(read('package.json')).version],
    ['src-tauri/tauri.conf.json', JSON.parse(read('src-tauri/tauri.conf.json')).version],
    ...[...lockVersions(read('Cargo.lock'))].map(([name, version]) => [`Cargo.lock:${name}`, version]),
  ]);
}

export function checkVersions(directory = root, expected) {
  const versions = versionState(directory);
  const version = expected ?? versions.get('Cargo.toml');
  parseVersion(version);
  for (const [file, value] of versions) {
    if (value !== version) throw new Error(`${file} has ${value}; expected ${version}. Run release:prepare.`);
  }
  for (const folder of ['src-tauri', 'crates/solar-agent-core', 'crates/solar-providers', 'crates/solar-tools']) {
    const manifest = fs.readFileSync(path.join(directory, folder, 'Cargo.toml'), 'utf8');
    if (!/^version\.workspace\s*=\s*true\s*$/m.test(manifest)) {
      throw new Error(`${folder}/Cargo.toml must inherit the workspace version.`);
    }
  }
  return version;
}

export function releaseNotes(directory, version) {
  parseVersion(version);
  const text = fs.readFileSync(path.join(directory, 'CHANGELOG.md'), 'utf8');
  const heading = `## [${version}]`;
  const lines = text.split(/\r?\n/);
  const indices = lines.flatMap((line, index) => line === heading ? [index] : []);
  if (indices.length !== 1) throw new Error(`Add one ${heading} section to CHANGELOG.md before tagging.`);
  const start = indices[0] + 1;
  const next = lines.findIndex((line, index) => index >= start && line.startsWith('## '));
  const notes = lines.slice(start, next < 0 ? undefined : next).join('\n').trim();
  if (!notes || !/^\s*[-*] \S/m.test(notes)) throw new Error(`Release ${version} needs changelog notes.`);
  return notes;
}

export function prepareVersion(directory, target) {
  const current = checkVersions(directory);
  let version = target;
  if (['major', 'minor', 'patch'].includes(target)) {
    const parts = parseVersion(current);
    const index = ['major', 'minor', 'patch'].indexOf(target);
    parts[index] += 1n;
    for (let i = index + 1; i < parts.length; i += 1) parts[i] = 0n;
    version = parts.join('.');
  }
  parseVersion(version);
  if (compareVersions(version, current) < 0) throw new Error('Release versions cannot move backwards.');
  const updates = new Map();
  for (const file of ['package.json', 'src-tauri/tauri.conf.json']) {
    const data = JSON.parse(fs.readFileSync(path.join(directory, file), 'utf8'));
    data.version = version;
    updates.set(file, `${JSON.stringify(data, null, 2)}\n`);
  }
  const cargo = fs.readFileSync(path.join(directory, 'Cargo.toml'), 'utf8');
  updates.set('Cargo.toml', cargo.replace(
    /(^\[workspace\.package\]\s*\n[\s\S]*?^version\s*=\s*")[^"]+("\s*$)/m,
    (_, before, after) => `${before}${version}${after}`,
  ));
  const lock = fs.readFileSync(path.join(directory, 'Cargo.lock'), 'utf8');
  updates.set('Cargo.lock', lock.split(/(?=^\[\[package\]\]$)/m).map((block) => {
    const name = block.match(/^name = "([^"]+)"$/m)?.[1];
    if (!localPackages.includes(name) || /^source = /m.test(block)) return block;
    return block.replace(/^version = "[^"]+"$/m, `version = "${version}"`);
  }).join(''));
  // All parsing/validation above happens before any files are changed.
  for (const [file, text] of updates) {
    const destination = path.join(directory, file);
    if (fs.readFileSync(destination, 'utf8') === text) continue;
    const temporary = `${destination}.solar-version-tmp`;
    fs.writeFileSync(temporary, text, { flag: 'wx', mode: fs.statSync(destination).mode });
    fs.renameSync(temporary, destination);
  }
  checkVersions(directory, version);
  return version;
}

function git(directory, ...args) {
  return execFileSync('git', args, { cwd: directory, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

export function createReleaseTag(directory, requested, push = false) {
  const version = checkVersions(directory, requested);
  releaseNotes(directory, version);
  if (git(directory, 'status', '--porcelain')) throw new Error('Commit all changes before tagging a release.');
  if (git(directory, 'branch', '--show-current') !== 'main') throw new Error('Create release tags from main.');
  const head = git(directory, 'rev-parse', 'HEAD');
  const remoteHead = git(directory, 'ls-remote', 'origin', 'refs/heads/main').split(/\s+/)[0];
  if (head !== remoteHead) throw new Error('Push main before tagging so the release points to reviewed remote source.');
  const local = git(directory, 'tag', '--list').split('\n');
  const remote = git(directory, 'ls-remote', '--tags', 'origin').split('\n').map((line) => line.split('\t')[1]?.replace(/^refs\/tags\//, '').replace(/\^\{\}$/, ''));
  for (const tag of new Set([...local, ...remote])) {
    if (!tag || !stableVersion.test(tag)) continue;
    if (compareVersions(version, tag) <= 0) throw new Error(`Use a version newer than existing tag ${tag}. Tags are never overwritten.`);
  }
  git(directory, 'tag', '-a', version, '-m', `Solar ${version}`);
  if (push) git(directory, 'push', 'origin', `refs/tags/${version}`);
  return version;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [command = 'check', argument, option, ...extra] = process.argv.slice(2);
    if (extra.length) throw new Error('Too many release arguments.');
    if (command === 'check') {
      if (option) throw new Error('Usage: release.mjs check [version]');
      console.log(`Solar ${checkVersions(root, argument)}: all versions match.`);
    } else if (command === 'prepare') {
      if (!argument || option) throw new Error('Usage: release.mjs prepare major|minor|patch|X.Y.Z');
      const version = prepareVersion(root, argument);
      console.log(`Prepared Solar ${version}. Add CHANGELOG.md notes, review, commit, and push main before tagging.`);
    } else if (command === 'tag') {
      if (option && option !== '--push') throw new Error('Usage: release.mjs tag [X.Y.Z] [--push]');
      const push = argument === '--push' || option === '--push';
      const version = createReleaseTag(root, argument === '--push' ? undefined : argument, push);
      console.log(push ? `Pushed ${version}; Gitea sync will trigger the signed GitHub release.` : `Created ${version}. Publish with: git push origin refs/tags/${version}`);
    } else throw new Error('Use check, prepare, or tag.');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
