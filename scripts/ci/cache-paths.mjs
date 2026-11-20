#!/usr/bin/env node
// Cache only unsigned compiler entries, public dependency downloads and pinned tools.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

function required(name) {
  const value = process.env[name];
  if (!value || /[\r\n]/.test(value)) throw new Error(`Missing or invalid ${name}.`);
  return value;
}

function output(file, text) {
  if (!path.isAbsolute(file)) throw new Error('Actions command files must be absolute.');
  fs.appendFileSync(file, text);
}

function directory(target) {
  if (fs.existsSync(target)) {
    const stat = fs.lstatSync(target);
    if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error('Use a real cache directory.');
  } else {
    fs.mkdirSync(target, { recursive: true, mode: 0o700 });
  }
}

function sccache(...args) {
  let command = process.env.SCCACHE_PATH || (process.platform === 'win32' ? 'sccache.exe' : 'sccache');
  if (process.platform === 'win32' && !path.extname(command)) command += '.exe';
  return execFileSync(command, args, { stdio: ['ignore', 'pipe', 'pipe'], encoding: 'utf8', timeout: 30_000 });
}

function main() {
  if (process.env.GITHUB_ACTIONS !== 'true' || required('GITHUB_REPOSITORY') !== 'rkw6086/solar') {
    throw new Error('Build caching is restricted to the Solar GitHub workflow.');
  }
  const mode = process.argv[2] || 'prepare';
  if (mode === 'activate') {
    const version = sccache('--version').trim();
    if (version !== 'sccache 0.19.0') throw new Error('Use the pinned sccache version.');
    let started = false;
    try {
      process.stdout.write(sccache('--start-server'));
      started = true;
      process.stdout.write(sccache('--zero-stats'));
      output(required('GITHUB_ENV'), 'SCCACHE_GHA_VERSION=\nRUSTC_WRAPPER=sccache\n');
    } catch (error) {
      if (started) {
        try { sccache('--stop-server'); } catch {}
      }
      throw error;
    }
    console.log(version);
    return;
  }
  if (mode === 'finish') {
    if (process.env.RUSTC_WRAPPER !== 'sccache') {
      console.log('Compiler cache unavailable; dependency and tool downloads can still be retained.');
      return;
    }
    try {
      process.stdout.write(sccache('--show-stats'));
    } finally {
      // Flush and close the local cache before archiving, especially on Windows.
      process.stdout.write(sccache('--stop-server'));
    }
    return;
  }
  if (mode !== 'prepare') throw new Error('Use prepare, activate, or finish.');

  const platforms = {
    darwin: { runner: 'macOS', arch: 'arm64' },
    linux: { runner: 'Linux', arch: 'x64' },
    win32: { runner: 'Windows', arch: 'x64' },
  };
  const platform = platforms[process.platform];
  if (!platform || process.arch !== platform.arch || required('RUNNER_OS') !== platform.runner) {
    throw new Error('Use a supported native GitHub build runner.');
  }
  const temporary = required('RUNNER_TEMP');
  if (!path.isAbsolute(temporary)) throw new Error('Use an absolute Actions temporary directory.');
  const root = path.join(temporary, 'solar-ci-cache-v1');
  directory(root);
  const compiler = path.join(root, 'sccache');
  const tools = path.join(root, 'tools');
  directory(compiler);
  directory(tools);

  // Stable compiler-facing paths are required: released sccache hashes CARGO_*
  // environment values. Run-number-specific paths would miss on every build.
  const cargoHome = process.platform === 'win32'
    ? path.join(required('SystemDrive') + path.sep, 'gca')
    : process.env.CARGO_HOME || path.join(os.homedir(), '.cargo');
  if (!path.isAbsolute(cargoHome) || /[\r\n]/.test(cargoHome)) throw new Error('Use an absolute Cargo home.');
  directory(cargoHome);
  const inputs = required('SOLAR_CACHE_INPUTS');
  const sha = required('SOLAR_RELEASE_SHA');
  if (!/^[a-f0-9]{64}$/.test(inputs) || !/^[a-f0-9]{40}$/.test(sha)) {
    throw new Error('Cache keys require validated build inputs and source identity.');
  }
  const identity = crypto.createHash('sha256').update([
    required('ImageOS'), required('ImageVersion'), 'rust-1.99.1', 'sccache-0.19.0', inputs,
  ].join('\0')).digest('hex');
  const prefix = `solar-build-v1-${process.platform}-${process.arch}-${identity}-`;
  const paths = [
    compiler, tools,
    path.join(cargoHome, 'registry', 'cache'),
    path.join(cargoHome, 'registry', 'index'),
    path.join(cargoHome, 'git', 'db'),
    path.join(cargoHome, 'git', 'checkouts'),
  ];
  output(required('GITHUB_ENV'), [
    `SOLAR_BUILD_CACHE_ROOT=${root}`,
    `SOLAR_CACHED_TOOLS_DIR=${tools}`,
    `SOLAR_CACHED_TARGET_DIR=${path.join(temporary, 'solar-build')}`,
    `CARGO_HOME=${cargoHome}`,
    `SCCACHE_DIR=${compiler}`,
    'SCCACHE_CACHE_SIZE=2G',
    'SCCACHE_GHA_ENABLED=false',
    'SCCACHE_GHA_VERSION=',
    'RUSTC_WRAPPER=',
    '',
  ].join('\n'));
  output(required('GITHUB_OUTPUT'), [
    `key=${prefix}${sha}`, `prefix=${prefix}`,
    'paths<<SOLAR_CACHE_PATHS', ...paths, 'SOLAR_CACHE_PATHS', '',
  ].join('\n'));
  console.log(`Prepared unsigned ${process.platform}/${process.arch} build cache (2 GiB compiler limit).`);
}

try {
  main();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
