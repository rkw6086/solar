#!/usr/bin/env node
// Native preview builds: no signing credentials or release mutation permissions.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { root, checkVersions } from '../release.mjs';

export const platforms = {
  win32: { name: 'windows', target: 'x86_64-pc-windows-msvc', bundles: 'nsis', packages: [['nsis', '.exe', 'x64-setup.exe']] },
  linux: { name: 'linux', target: 'x86_64-unknown-linux-gnu', bundles: 'deb,appimage', packages: [['deb', '.deb', 'amd64.deb'], ['appimage', '.AppImage', 'x86_64.AppImage']] },
};

export function validateSource({ env = process.env, platform = process.platform, arch = process.arch,
  directory = root, git = (...args) => execFileSync('git', args, { cwd: directory, encoding: 'utf8' }).trim() } = {}) {
  const config = platforms[platform];
  if (!config || arch !== 'x64') throw new Error('Use a native Windows or Linux x64 runner.');
  if (env.GITHUB_ACTIONS !== 'true' || env.GITHUB_SERVER_URL !== 'https://github.com'
      || env.GITHUB_REPOSITORY !== 'rkw6086/solar') throw new Error('Use the Solar GitHub repository.');
  const event = env.GITHUB_EVENT_NAME;
  const channel = env.SOLAR_BUILD_CHANNEL || 'ci';
  const version = checkVersions(directory);
  if (channel === 'ci') {
    if (!(['push', 'workflow_dispatch'].includes(event) && env.GITHUB_REF === 'refs/heads/main')
        && !(event === 'pull_request' && env.GITHUB_BASE_REF === 'main')) {
      throw new Error('Build a main push, a pull request into main, or dispatch on main.');
    }
  } else if (channel === 'release') {
    if (event !== 'push' || env.GITHUB_REF !== `refs/tags/${version}`) throw new Error('Release builds require the exact version tag.');
  } else if (channel === 'beta') {
    if (!['schedule', 'workflow_dispatch'].includes(event) || env.GITHUB_REF !== 'refs/heads/main'
        || env.SOLAR_BETA_ID !== env.SOLAR_RELEASE_SHA) throw new Error('Beta builds require validated main source.');
  } else throw new Error('Unknown desktop build channel.');
  if (channel !== 'ci' && env.SOLAR_RELEASE_VERSION !== version) {
    throw new Error('The validated release version differs from the checkout.');
  }
  const sha = git('rev-parse', 'HEAD');
  if (!/^[0-9a-f]{40}$/.test(sha) || !/^[0-9a-f]{40}$/.test(env.GITHUB_SHA ?? '')
      || sha !== git('rev-parse', `${env.GITHUB_SHA}^{commit}`) || sha !== env.SOLAR_RELEASE_SHA
      || git('status', '--porcelain', '--untracked-files=all')) throw new Error('Build the clean triggering source.');
  if (!/^\d+-\d+$/.test(env.SOLAR_JOB_ID ?? '') || !path.isAbsolute(env.RUNNER_TEMP ?? '')) {
    throw new Error('Missing job-owned build directory.');
  }
  return { ...config, sha, version, channel: channel === 'ci' ? 'preview' : channel,
    targetDir: path.join(env.RUNNER_TEMP, 'solar-build'),
    artifactsDir: path.join(env.RUNNER_TEMP, `solar-desktop-${env.SOLAR_JOB_ID}-${config.name}`) };
}

export function collectPackages({ config }) {
  if (fs.existsSync(config.artifactsDir)) throw new Error('The artifact directory must be new.');
  const inputs = config.packages.map(([folder, extension, suffix]) => {
    const bundle = path.join(config.targetDir, config.target, 'release', 'bundle', folder);
    const matches = fs.readdirSync(bundle).filter(name => name.endsWith(extension));
    if (matches.length !== 1) throw new Error(`Expected one ${folder} package, found ${matches.length}.`);
    const source = path.join(bundle, matches[0]);
    const stat = fs.lstatSync(source);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0 || stat.size > 1024 ** 3) {
      throw new Error(`Invalid ${folder} package.`);
    }
    const identity = config.channel === 'beta' ? `beta_${config.sha}` : config.version;
    const name = `Solar_${identity}_${suffix}`;
    return { source, name };
  });
  fs.mkdirSync(config.artifactsDir, { mode: 0o700 });
  const checksums = [];
  for (const { source, name } of inputs) {
    const destination = path.join(config.artifactsDir, name);
    fs.copyFileSync(source, destination, fs.constants.COPYFILE_EXCL);
    const digest = crypto.createHash('sha256').update(fs.readFileSync(destination)).digest('hex');
    checksums.push(`${digest}  ${name}`);
  }
  fs.writeFileSync(path.join(config.artifactsDir, 'SHA256SUMS'), `${checksums.join('\n')}\n`, { flag: 'wx' });
  fs.writeFileSync(path.join(config.artifactsDir, 'BUILD-INFO.json'), `${JSON.stringify({
    version: config.version, source: config.sha, target: config.target, channel: config.channel || 'preview', signed: false,
    packages: inputs.map(({ name }) => name),
  }, null, 2)}\n`, { flag: 'wx' });
  return inputs.map(({ name }) => path.join(config.artifactsDir, name));
}

function main() {
  const config = validateSource();
  if (process.argv[2] === 'prepare') {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `platform=${config.name}\ntarget=${config.target}\nartifacts_dir=${config.artifactsDir}\n`);
    console.log(`Preview build: Solar ${config.version}, ${config.target}, source ${config.sha}.`);
    return;
  }
  if (process.argv[2] !== 'build') throw new Error('Use prepare or build.');
  const env = { ...process.env, CARGO_TARGET_DIR: config.targetDir };
  const run = (command, args) => execFileSync(command, args, { cwd: root, env, stdio: 'inherit' });
  run('cargo', ['fmt', '--all', '--', '--check']);
  run(process.execPath, [path.join(root, 'node_modules/@tauri-apps/cli/tauri.js'), 'build', '--ci',
    '--target', config.target, '--bundles', config.bundles, '--no-sign', '--', '--locked']);
  for (const file of collectPackages({ config })) console.log(`Prepared ${path.basename(file)}.`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
