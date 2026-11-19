#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkVersions, root } from '../release.mjs';

const COMMIT = /^[0-9a-f]{40}$/;
const MARKER = /<!-- solar-beta:v1 commit=([0-9a-f]{40})(?: [^>\r\n]*)? -->/g;

export function betaContext(env) {
  if (!['schedule', 'workflow_dispatch'].includes(env.GITHUB_EVENT_NAME)
      || env.GITHUB_REF !== 'refs/heads/main') {
    throw new Error('Beta signing requires a scheduled or manual run on main.');
  }
  const repository = env.GITEA_REPOSITORY;
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository ?? '')
      || repository.split('/').some((part) => part === '.' || part === '..')
      || env.GITHUB_REPOSITORY !== repository) {
    throw new Error('Invalid beta workflow repository.');
  }
  const server = new URL(env.GITEA_SERVER_URL);
  if (server.protocol !== 'https:' || server.username || server.password
      || server.search || server.hash || server.pathname !== '/') {
    throw new Error('Gitea must use an HTTPS server URL without credentials or a path.');
  }
  if (!env.GITEA_TOKEN || /\s/.test(env.GITEA_TOKEN)) throw new Error('Missing workflow token.');
  return { server, repository };
}

export function previousBeta(current) {
  if (!current) return '';
  const matches = typeof current.body === 'string' ? [...current.body.matchAll(MARKER)] : [];
  if (matches.length !== 1 || current.tag_name !== 'beta' || current.draft || !current.prerelease) {
    throw new Error('The existing beta does not match Solar’s rolling release; left unchanged.');
  }
  return matches[0][1];
}

export function betaNeedsBuild(eventName, current, tag, sha) {
  const previous = previousBeta(current);
  const names = new Set([`Solar_beta_${sha}_aarch64.dmg`, 'SHA256SUMS']);
  const complete = current?.assets?.length === 2 && current.assets.every((asset) =>
    names.delete(asset.name) && Number.isSafeInteger(asset.size) && asset.size > 0
    && asset.size <= (asset.name === 'SHA256SUMS' ? 4096 : 512 * 1024 * 1024))
    && names.size === 0;
  return !(eventName === 'schedule' && previous === sha && complete
    && tag?.commit?.sha === sha && current.target_commitish === sha);
}

export function betaNotes({ sha, previous, eventName, git, server, repository }) {
  if (previous === sha) {
    return eventName === 'workflow_dispatch'
      ? 'Manually rebuilt main with no source changes since the previous beta.'
      : 'No source changes since the previous beta build; restoring the rolling beta downloads.';
  }
  let range = sha;
  let introduction = 'Initial rolling beta from main.';
  if (previous) {
    let ancestor = false;
    try { git('merge-base', '--is-ancestor', previous, sha); ancestor = true; } catch { /* Use recent history after a main rewrite. */ }
    if (ancestor) {
      range = `${previous}..${sha}`;
      introduction = `Changes since the previous beta (\`${previous.slice(0, 12)}\`):`;
    } else {
      introduction = 'Main history changed since the previous beta. Recent commits:';
    }
  }
  const commits = git('log', '--format=%h %s', '--max-count=201', range, '--').split('\n').filter(Boolean);
  const lines = [];
  let bytes = 0;
  for (const line of commits.slice(0, 200)) {
    const bullet = `- ${line.replace(/[\x00-\x1f\x7f]/g, ' ').slice(0, 500)}`;
    bytes += Buffer.byteLength(bullet, 'utf8') + 1;
    if (bytes > 96 * 1024) break;
    lines.push(bullet);
  }
  let notes = `${introduction}\n\n${lines.join('\n') || '- No source changes.'}`;
  if (commits.length > lines.length) notes += `\n\nShowing the latest ${lines.length} commits within the release-note limit.`;
  if (previous) notes += `\n\n[Compare changes](${server.origin}/${repository}/compare/${previous}...${sha})`;
  return notes;
}

export async function checkBetaBuild({ env = process.env, directory = root, git: suppliedGit, api: suppliedApi } = {}) {
  const { server, repository } = betaContext(env);
  const git = suppliedGit ?? ((...args) => execFileSync('git', args, {
    cwd: directory, encoding: 'utf8', maxBuffer: 2 * 1024 * 1024,
  }).trim());
  const api = suppliedApi ?? (async (suffix, allowMissing = false) => {
    const response = await fetch(new URL(`/api/v1/repos/${repository}${suffix}`, server), {
      headers: { Authorization: `token ${env.GITEA_TOKEN}`, Accept: 'application/json' },
      redirect: 'error', signal: AbortSignal.timeout(30_000),
    });
    if (response.status === 404 && allowMissing) return null;
    if (!response.ok) throw new Error(`Beta preflight API returned HTTP ${response.status}.`);
    return response.json();
  });
  const sha = git('rev-parse', 'HEAD');
  if (!COMMIT.test(sha) || git('status', '--porcelain', '--untracked-files=all')
      || git('rev-parse', 'refs/remotes/origin/main') !== sha) {
    throw new Error('Beta builds require a clean checkout of main.');
  }
  // Scheduled context SHAs may be old. The workflow checks out main explicitly;
  // validate that checkout against the live branch, not GITHUB_SHA.
  const main = await api('/branches/main');
  if (main.commit?.id !== sha) throw new Error('Main advanced during checkout; rerun the beta workflow.');
  const current = await api('/releases/tags/beta', true);
  const previous = previousBeta(current);
  const version = checkVersions(directory);
  const tag = current && previous === sha && env.GITHUB_EVENT_NAME === 'schedule'
    ? await api('/tags/beta', true) : null;
  if (!betaNeedsBuild(env.GITHUB_EVENT_NAME, current, tag, sha)) {
    fs.appendFileSync(env.GITHUB_OUTPUT, 'build=false\n');
    console.log(`Main remains at ${sha}; the published beta is current. Nothing to build.`);
    return { build: false, sha, version, previous };
  }
  const temporary = env.RUNNER_TEMP;
  const job = env.SOLAR_JOB_ID;
  if (!temporary || !path.isAbsolute(temporary) || !/^[0-9]+-[0-9]+$/.test(job ?? '')) {
    throw new Error('Missing job-owned temporary directory or job identity.');
  }
  const notesDirectory = path.join(temporary, `solar-notes-${job}`);
  fs.mkdirSync(notesDirectory, { mode: 0o700 });
  const notesFile = path.join(notesDirectory, 'release-notes.md');
  fs.writeFileSync(notesFile, `${betaNotes({ sha, previous, eventName: env.GITHUB_EVENT_NAME, git, server, repository })}\n`,
    { mode: 0o600, flag: 'wx' });
  fs.appendFileSync(env.GITHUB_OUTPUT, `build=true\nversion=${version}\nsha=${sha}\n`);
  fs.appendFileSync(env.GITHUB_ENV,
    `SOLAR_BUILD_CHANNEL=beta\nSOLAR_RELEASE_VERSION=${version}\nSOLAR_RELEASE_SHA=${sha}\nSOLAR_BETA_ID=${sha}\n`
    + `SOLAR_PREVIOUS_BETA_SHA=${previous}\nSOLAR_RELEASE_NOTES_FILE=${notesFile}\n`);
  console.log(`Validated Solar beta at ${sha}; previous published beta: ${previous || 'none'}.`);
  return { build: true, sha, version, previous };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  checkBetaBuild().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
