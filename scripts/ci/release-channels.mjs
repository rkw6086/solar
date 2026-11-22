// Pure release-channel helpers shared by GitHub validation and tests.
const MARKER = /<!-- solar-beta:v1 commit=([0-9a-f]{40})(?: [^>\r\n]*)? -->/g;

export function previousBeta(current) {
  if (!current) return '';
  const matches = typeof current.body === 'string' ? [...current.body.matchAll(MARKER)] : [];
  if (matches.length !== 1 || current.tag_name !== 'beta' || current.draft || !current.prerelease) {
    throw new Error('The existing beta does not match Solar’s rolling release; left unchanged.');
  }
  return matches[0][1];
}

export const PACKAGE_MARKER = '<!-- solar-packages:v1 platforms=macos,windows,linux -->';

export function packageNames(identity) {
  return [`Solar_${identity}_aarch64.dmg`, `Solar_${identity}_x64-setup.exe`,
    `Solar_${identity}_amd64.deb`, `Solar_${identity}_x86_64.AppImage`];
}

export function completePackageInventory(current, identity, { allowLegacy = false } = {}) {
  const packages = packageNames(identity);
  const markerCount = (current?.body ?? '').split(PACKAGE_MARKER).length - 1;
  if (markerCount > 1) return false;
  const legacy = allowLegacy && markerCount === 0 && current?.assets?.length === 2;
  const names = new Set([...(legacy ? packages.slice(0, 1) : packages), 'SHA256SUMS']);
  return current?.assets?.length === names.size && current.assets.every((asset) =>
    names.delete(asset.name) && Number.isSafeInteger(asset.size) && asset.size > 0
    && asset.size <= (asset.name === 'SHA256SUMS' ? 4096 : 512 * 1024 * 1024))
    && names.size === 0;
}

export function betaNeedsBuild(eventName, current, tag, sha) {
  const previous = previousBeta(current);
  return !(eventName === 'schedule' && previous === sha && completePackageInventory(current, `beta_${sha}`)
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
