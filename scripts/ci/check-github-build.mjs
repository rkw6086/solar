#!/usr/bin/env node
// Resolve source and channel before a platform job can receive signing keys.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { root, checkVersions, compareVersions, releaseNotes } from '../release.mjs';
import { betaNeedsBuild, betaNotes, completePackageInventory, previousBeta } from './release-channels.mjs';

const REPOSITORY = 'rkw6086/solar';
const SHA = /^[0-9a-f]{40}$/;
const VERSION = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/;

export function githubContext(env, event) {
  if (env.GITHUB_SERVER_URL !== 'https://github.com' || env.GITHUB_REPOSITORY !== REPOSITORY
      || (env.GITHUB_API_URL && env.GITHUB_API_URL !== 'https://api.github.com')) {
    throw new Error('Use the Solar GitHub repository.');
  }
  const kind = env.GITHUB_EVENT_NAME;
  const ref = env.GITHUB_REF;
  if (kind === 'pull_request') {
    if (event.pull_request?.base?.ref !== 'main' || event.pull_request?.base?.repo?.full_name !== REPOSITORY) {
      throw new Error('Pull request validation must target Solar main.');
    }
    return 'ci';
  }
  if (kind === 'push' && ref === 'refs/heads/main' && !event.deleted) return 'ci';
  if (['schedule', 'workflow_dispatch'].includes(kind) && ref === 'refs/heads/main') return 'beta';
  if (kind === 'push' && ref?.startsWith('refs/tags/') && VERSION.test(ref.slice(10))
      && !event.deleted && !/^0+$/.test(event.after ?? '')) return 'release';
  throw new Error('Unsupported build event or ref.');
}

export async function checkGitHubBuild({ env = process.env, directory = root, event: suppliedEvent,
  git: suppliedGit, api: suppliedApi, versions = checkVersions, notesForVersion = releaseNotes } = {}) {
  const event = suppliedEvent ?? JSON.parse(fs.readFileSync(env.GITHUB_EVENT_PATH, 'utf8'));
  const channel = githubContext(env, event);
  const git = suppliedGit ?? ((...args) => execFileSync('git', args, {
    cwd: directory, encoding: 'utf8', timeout: 60_000, maxBuffer: 2 * 1024 * 1024,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim());
  const sha = git('rev-parse', 'HEAD');
  let version = versions(directory);
  if (!SHA.test(sha) || git('status', '--porcelain', '--untracked-files=all')) {
    throw new Error('Build preparation requires a clean source checkout.');
  }
  if (channel === 'ci' && sha !== env.GITHUB_SHA) throw new Error('CI checkout differs from its triggering source.');
  let previous = '';
  let previousId = '';
  let notes = '';
  if (channel !== 'ci') {
    const token = env.GITHUB_TOKEN;
    if (!token || /\s/.test(token)) throw new Error('Release lookup requires an authenticated GitHub workflow token.');
    const api = suppliedApi ?? (async (suffix, missing = false) => {
      const response = await fetch(`https://api.github.com/repos/${REPOSITORY}${suffix}`, {
        headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2026-03-10' },
        redirect: 'error', signal: AbortSignal.timeout(30_000),
      });
      if (response.status === 404 && missing) return null;
      if (!response.ok) throw new Error(`Build preflight API returned HTTP ${response.status}.`);
      return response.json();
    });
    const releases = [];
    for (let page = 1; page <= 100; page += 1) {
      const values = await api(`/releases?per_page=100&page=${page}`);
      if (!Array.isArray(values) || values.some(value => !value || typeof value !== 'object' || Array.isArray(value))) {
        throw new Error('Cannot inspect release history.');
      }
      releases.push(...values);
      if (values.length < 100) break;
      if (page === 100) throw new Error('Release history exceeded its safe bound.');
    }
    const releaseForTag = async (tag) => {
      const matches = releases.filter(release => release.tag_name === tag);
      if (matches.length > 1) throw new Error('Multiple releases claim the same tag.');
      if (!matches.length) return null;
      const id = matches[0].id;
      if (!Number.isSafeInteger(id) || id <= 0) throw new Error('Release identity is invalid.');
      const result = await api(`/releases/${id}`);
      if (result.id !== id || result.tag_name !== tag) throw new Error('Release identity changed during lookup.');
      return result;
    };
    const tagCommit = async (tag, missing = false) => {
      let value = await api(`/git/ref/tags/${encodeURIComponent(tag)}`, missing);
      if (!value) return null;
      if (value.ref !== `refs/tags/${tag}`) throw new Error('Unexpected tag identity.');
      let object = value.object;
      for (let depth = 0; depth < 8 && object?.type === 'tag'; depth += 1) object = (await api(`/git/tags/${object.sha}`)).object;
      if (object?.type !== 'commit' || !SHA.test(object.sha)) throw new Error('Tag does not identify a bounded source commit.');
      return object.sha;
    };
    if (channel === 'beta') {
      if (git('rev-parse', 'refs/remotes/origin/main') !== sha || (await api('/branches/main')).commit?.sha !== sha) {
        throw new Error('Beta checkout must match current GitHub main.');
      }
      const current = await releaseForTag('beta');
      previous = previousBeta(current);
      previousId = current ? String(current.id) : '';
      const tag = await tagCommit('beta', true);
      if (current && tag !== previous) throw new Error('Published beta and its source ref disagree.');
      if (!current && tag) {
        // A verified stage may outlive deletion of the previous beta record.
        // Recover only refs explicitly bound by one of our published stages.
        const candidates = releases.filter(release => {
          const stage = /^beta-stage-([0-9a-f]{40})-[0-9a-f]{16}$/.exec(release.tag_name ?? '');
          const transition = /<!-- solar-beta-transition:v1 previous=([0-9a-f]{40}|none) release=([1-9][0-9]*|none) -->/.exec(release.body ?? '');
          if (!stage || !transition || release.draft || !release.prerelease || release.target_commitish !== stage[1]
              || !Number.isSafeInteger(release.id) || release.id <= 0
              || (release.body ?? '').split('<!-- solar-beta-transition:').length !== 2
              || (release.body ?? '').split('<!-- solar-beta:').length !== 2
              || !(release.body ?? '').includes(`<!-- solar-beta:v1 commit=${stage[1]} -->`)
              || ![stage[1], transition[1]].includes(tag)) return false;
          return completePackageInventory(release, `beta_${stage[1]}`, { allowLegacy: true });
        }).sort((left, right) => right.id - left.id);
        if (!candidates.length) throw new Error('An unowned beta ref exists without a release.');
        if (await tagCommit(candidates[0].tag_name) !== candidates[0].target_commitish) {
          throw new Error('Interrupted beta staging ref no longer matches its source.');
        }
        previous = tag;
        previousId = String(candidates[0].id);
      }
      if (!betaNeedsBuild(env.GITHUB_EVENT_NAME, current, tag ? { commit: { sha: tag } } : null, sha)) {
        fs.appendFileSync(env.GITHUB_OUTPUT, 'build=false\nchannel=beta\n');
        console.log(`Main remains at ${sha}; the published beta is current.`);
        return { build: false, channel, version, sha };
      }
      notes = betaNotes({ sha, previous, eventName: env.GITHUB_EVENT_NAME, git,
        server: new URL('https://github.com'), repository: REPOSITORY });
    } else {
      version = env.GITHUB_REF.slice('refs/tags/'.length);
      versions(directory, version);
      if (git('rev-parse', `${env.GITHUB_REF}^{commit}`) !== sha || await tagCommit(version) !== sha
          || git('rev-parse', `${env.GITHUB_SHA}^{commit}`) !== sha) throw new Error('Release tag or source changed.');
      git('merge-base', '--is-ancestor', sha, 'refs/remotes/origin/main');
      const knownTags = git('tag', '--list').split('\n').filter(tag => VERSION.test(tag));
      const publishedTags = releases.filter(release => !release.draft && !release.prerelease && VERSION.test(release.tag_name ?? ''))
        .map(release => release.tag_name);
      if ([...knownTags, ...publishedTags].some(tag => compareVersions(tag, version) > 0)) {
        throw new Error('A newer stable version already exists; stale signing stopped.');
      }
      const current = await releaseForTag(version);
      if (current && !current.draft) {
        const marker = `<!-- solar-release:v1 tag=${version} commit=${sha} -->`;
        if (current.prerelease || current.target_commitish !== sha || !current.body?.includes(marker)) {
          throw new Error('An existing release does not match this source; left unchanged.');
        }
        fs.appendFileSync(env.GITHUB_OUTPUT, 'build=false\nchannel=release\n');
        console.log(`Solar ${version} is already published; its files remain unchanged.`);
        return { build: false, channel, version, sha };
      }
      notes = notesForVersion(directory, version);
    }
    const temporary = env.RUNNER_TEMP;
    const job = env.SOLAR_JOB_ID;
    if (!temporary || !path.isAbsolute(temporary) || !/^[0-9]+-[0-9]+$/.test(job ?? '')) throw new Error('Missing job-owned notes directory identity.');
    const folder = path.join(temporary, `solar-notes-${job}`);
    fs.mkdirSync(folder, { mode: 0o700 });
    const file = path.join(folder, 'release-notes.md');
    if (!notes.trim() || Buffer.byteLength(notes) > 128 * 1024) throw new Error('Release notes exceeded their safe bound.');
    fs.writeFileSync(file, `${notes.trim()}\n`, { mode: 0o600, flag: 'wx' });
    fs.appendFileSync(env.GITHUB_OUTPUT, `notes_file=${file}\n`);
    fs.appendFileSync(env.GITHUB_ENV, `SOLAR_RELEASE_NOTES_FILE=${file}\n`);
  }
  fs.appendFileSync(env.GITHUB_OUTPUT,
    `build=true\nchannel=${channel}\nversion=${version}\nsha=${sha}\nbeta_id=${channel === 'beta' ? sha : ''}\nprevious_beta_sha=${previous}\nprevious_beta_release_id=${previousId}\n`);
  fs.appendFileSync(env.GITHUB_ENV,
    `SOLAR_BUILD_CHANNEL=${channel}\nSOLAR_RELEASE_VERSION=${version}\nSOLAR_RELEASE_SHA=${sha}\nSOLAR_BETA_ID=${channel === 'beta' ? sha : ''}\nSOLAR_PREVIOUS_BETA_SHA=${previous}\nSOLAR_PREVIOUS_BETA_RELEASE_ID=${previousId}\n`);
  console.log(`Validated Solar ${channel} ${version} at ${sha}.`);
  return { build: true, channel, version, sha, previous, previousId };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  checkGitHubBuild().catch(error => { console.error(error.message); process.exitCode = 1; });
}
