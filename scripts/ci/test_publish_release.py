"""Exercise channel replacement and retention without network or real credentials."""
import copy
from dataclasses import replace
import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
import urllib.error
import urllib.parse

SPEC = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish-release.py"))
MODULE = importlib.util.module_from_spec(SPEC)
import sys
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class FakeGitea:
    def __init__(self, config):
        self.config = config
        self.releases = {}
        self.asset_lists = {}
        self.tags = {config.version: config.commit}
        self.head = config.commit
        self.calls = []
        self.downloads = []
        self.events = []
        self.verified_asset_digests = {}
        self.artifacts = {}
        self.runs = {}
        self.next_release = 10
        self.next_asset = 1
        self.tag_reads = 0
        self.move_tag = False
        self.fail_upload_after_save = None
        self.fail_promote_after_save = False
        self.fail_download = None
        self.before_request = None
        self.unindexed_tags = set()

    @property
    def release(self):
        return next((r for r in self.releases.values() if r['tag_name'] == self.config.tag), None)

    @release.setter
    def release(self, value):
        self.releases[value['id']] = value
        self.asset_lists.setdefault(value['id'], [])
        self.next_release = max(self.next_release, value['id'] + 1)

    @property
    def stored(self):
        return self.asset_lists[self.release['id']]

    def generated_release(self, draft=True):
        return {'id': 10, 'draft': draft, 'prerelease': False,
                'tag_name': self.config.tag, 'target_commitish': self.config.commit,
                'body': f'<!-- solar-release:v1 tag={self.config.tag} commit={self.config.commit} -->'}

    def store(self, path, data=None, release_id=None):
        payload = path.read_bytes() if data is None else data
        release_id = self.release['id'] if release_id is None else release_id
        return self.store_bytes(release_id, path.name, payload)

    def store_bytes(self, release_id, name, payload):
        asset = {'id': self.next_asset, 'name': name, 'size': len(payload),
                 'browser_download_url': 'https://gitea.example/asset/' + str(self.next_asset),
                 'payload': payload}
        self.next_asset += 1
        self.asset_lists.setdefault(release_id, []).append(asset)
        return asset

    def add_release(self, tag, commit, channel='release', draft=False, data=b'old verified image'):
        config = replace(self.config, tag=tag, commit=commit, channel=channel)
        release = {'id': self.next_release, 'tag_name': tag, 'target_commitish': commit,
                   'draft': draft, 'prerelease': channel == 'beta',
                   'body': MODULE.release_body(config, '- Test release.')}
        self.next_release += 1
        self.releases[release['id']] = release
        self.tags[tag] = commit
        name = f'Solar_beta_{commit}_aarch64.dmg' if channel == 'beta' else f'Solar_{tag}_aarch64.dmg'
        self.store_bytes(release['id'], name, data)
        self.store_bytes(release['id'], 'SHA256SUMS',
                         (hashlib.sha256(data).hexdigest() + '  ' + name + '\n').encode())
        return release

    def request(self, method, path, body=None, upload=None, missing_ok=False):
        self.calls.append((method, path))
        self.events.append((method, path))
        if self.before_request:
            self.before_request(method, path, body)
        parsed = urllib.parse.urlsplit(path)
        route = urllib.parse.unquote(parsed.path)
        params = urllib.parse.parse_qs(parsed.query)
        parts = route.strip('/').split('/')
        def page(values):
            limit = int(params.get('limit', ['50'])[0])
            offset = (int(params.get('page', ['1'])[0]) - 1) * limit
            return copy.deepcopy(values[offset:offset + limit])
        def missing():
            if missing_ok:
                return None
            raise MODULE.MissingResource('Missing fake resource')
        if route == '/branches/main':
            return {'commit': {'id': self.head}}
        if parts[0] == 'tags':
            tag = parts[1]
            if tag not in self.tags:
                return missing()
            if method == 'DELETE':
                if tag in self.unindexed_tags:
                    raise MODULE.MissingResource('Unindexed fake tag')
                del self.tags[tag]
                return None
            self.tag_reads += 1
            sha = 'b' * 40 if self.move_tag and self.tag_reads > 1 else self.tags[tag]
            return {'name': tag, 'commit': {'sha': sha}}
        if route == '/releases' and method == 'GET':
            return page(list(self.releases.values()))
        if route == '/releases' and method == 'POST':
            self.tags.setdefault(body['tag_name'], body['target_commitish'])
            value = {'id': self.next_release, **copy.deepcopy(body)}
            self.next_release += 1
            self.releases[value['id']] = value
            self.asset_lists[value['id']] = []
            return copy.deepcopy(value)
        if parts[:2] == ['releases', 'tags']:
            value = next((r for r in self.releases.values() if r['tag_name'] == parts[2]), None)
            return copy.deepcopy(value) if value else missing()
        if parts[0] == 'releases' and len(parts) >= 2:
            rid = int(parts[1])
            if rid not in self.releases:
                return missing()
            if len(parts) == 3 and parts[2] == 'assets':
                if method == 'GET':
                    return page([{k:v for k,v in a.items() if k != 'payload'} for a in self.asset_lists[rid]])
                if method == 'POST':
                    asset = self.store(upload, release_id=rid)
                    if self.fail_upload_after_save == upload.name:
                        self.fail_upload_after_save = None
                        raise MODULE.PublishError('Simulated lost upload response')
                    return {k:v for k,v in asset.items() if k != 'payload'}
            if method == 'GET':
                return copy.deepcopy(self.releases[rid])
            if method == 'DELETE':
                del self.releases[rid]
                del self.asset_lists[rid]
                return None
            if method == 'PATCH':
                if 'tag_name' in body:
                    self.tags[body['tag_name']] = body['target_commitish']
                self.releases[rid].update(copy.deepcopy(body))
                if body.get('tag_name') == 'beta' and self.fail_promote_after_save:
                    self.fail_promote_after_save = False
                    raise MODULE.PublishError('Simulated lost promotion response')
                return copy.deepcopy(self.releases[rid])
        if parts[:2] == ['actions', 'artifacts']:
            if len(parts) == 2:
                return {'artifacts': page(list(self.artifacts.values()))}
            aid = int(parts[2])
            if aid not in self.artifacts:
                return missing()
            if method == 'DELETE':
                del self.artifacts[aid]
                return None
            return copy.deepcopy(self.artifacts[aid])
        if parts[:2] == ['actions', 'runs']:
            return copy.deepcopy(self.runs[int(parts[2])])
        raise AssertionError((method, path))

    def payload(self, url):
        return next(a['payload'] for assets in self.asset_lists.values()
                    for a in assets if a['browser_download_url'] == url)

    def download_digest(self, url, expected_size):
        if self.fail_download == url:
            raise MODULE.PublishError('Simulated download failure')
        payload = self.payload(url)
        assert expected_size == len(payload)
        self.downloads.append(url)
        self.events.append(('download', url))
        return hashlib.sha256(payload).hexdigest()

    def download_bytes(self, url, limit):
        payload = self.payload(url)
        assert len(payload) <= limit
        self.downloads.append(url)
        self.events.append(('download', url))
        return payload


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        self.config = MODULE.Config('0.1.3', '0.1.3', self.directory,
                                    'https://gitea.example', 'internal/solar-chat', 'a' * 40, 'dummy-token',
                                    notes='- Signed macOS release.')
        self.make_files(self.config)
        self.client = FakeGitea(self.config)

    def tearDown(self):
        self.temp.cleanup()

    def make_files(self, config, data=b'signed-and-notarized-test-image'):
        identity = 'beta_' + config.commit if config.channel == 'beta' else config.version
        self.dmg = self.directory / f'Solar_{identity}_aarch64.dmg'
        self.dmg.write_bytes(data)
        self.sums = self.directory / 'SHA256SUMS'
        self.sums.write_text(hashlib.sha256(data).hexdigest() + '  ' + self.dmg.name + '\n')

    def beta(self, previous='b' * 40):
        self.config = replace(self.config, tag='beta', channel='beta', previous_beta=previous)
        self.make_files(self.config)
        self.client.config = self.config
        if previous:
            return self.client.add_release('beta', previous, channel='beta')

    def publish(self):
        return MODULE.publish(self.config, self.client)

    def mutations(self):
        return [call for call in self.client.calls if call[0] != 'GET']

    def test_creates_draft_verifies_both_assets_then_publishes(self):
        url = self.publish()
        self.assertEqual(url, 'https://gitea.example/internal/solar-chat/releases/tag/0.1.3')
        self.assertFalse(self.client.release['draft'])
        self.assertIn('- Signed macOS release.', self.client.release['body'])
        self.assertEqual([a['name'] for a in self.client.stored], [self.dmg.name, 'SHA256SUMS'])
        self.assertEqual([call[0] for call in self.mutations()], ['POST', 'POST', 'POST', 'PATCH'])
        publication = self.client.events.index(('PATCH', '/releases/10'))
        self.assertEqual(sum(kind == 'download' for kind, _ in self.client.events[:publication]), 2)

    def test_bad_local_checksum_never_calls_server(self):
        self.sums.write_text('0' * 64 + '  ' + self.dmg.name + '\n')
        with self.assertRaisesRegex(MODULE.PublishError, 'SHA256SUMS'):
            self.publish()
        self.assertEqual(self.client.calls, [])

    def test_empty_release_notes_never_call_server(self):
        with self.assertRaisesRegex(MODULE.PublishError, 'notes'):
            MODULE.publish(replace(self.config, notes='  '), self.client)
        self.assertEqual(self.client.calls, [])

    def test_rejects_symlink_asset(self):
        original = self.directory / 'other.dmg'
        self.dmg.rename(original)
        self.dmg.symlink_to(original)
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertEqual(self.client.calls, [])

    def test_resumes_matching_draft_without_duplicate_asset(self):
        self.client.release = self.client.generated_release()
        self.client.store(self.dmg)
        self.publish()
        self.assertEqual(len(self.client.stored), 2)
        self.assertEqual(sum('/assets?' in path for _, path in self.mutations()), 1)

    def test_lost_upload_response_resumes_existing_bytes(self):
        self.client.fail_upload_after_save = self.sums.name
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertTrue(self.client.release['draft'])
        self.assertEqual(len(self.client.stored), 2)
        self.publish()
        self.assertFalse(self.client.release['draft'])
        self.assertEqual(len(self.client.stored), 2)

    def test_foreign_draft_remains_untouched(self):
        self.client.release = self.client.generated_release()
        self.client.release['body'] = 'Manually prepared release'
        with self.assertRaisesRegex(MODULE.PublishError, 'left unchanged'):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_mismatching_asset_is_not_replaced(self):
        self.client.release = self.client.generated_release()
        self.client.store(self.dmg, b'x' * self.dmg.stat().st_size)
        with self.assertRaisesRegex(MODULE.PublishError, 'checksum differs'):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_duplicate_assets_remain_unpublished(self):
        self.client.release = self.client.generated_release()
        self.client.store(self.dmg)
        self.client.store(self.dmg)
        with self.assertRaisesRegex(MODULE.PublishError, 'duplicate'):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_completed_matching_release_is_idempotent(self):
        self.client.release = self.client.generated_release(draft=False)
        self.client.store(self.dmg)
        self.client.store(self.sums)
        self.publish()
        self.assertEqual(self.mutations(), [])

    def test_published_incomplete_release_is_never_modified(self):
        self.client.release = self.client.generated_release(draft=False)
        self.client.store(self.dmg)
        with self.assertRaisesRegex(MODULE.PublishError, 'incomplete|missing'):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_tag_moved_during_upload_leaves_draft(self):
        self.client.move_tag = True
        with self.assertRaisesRegex(MODULE.PublishError, 'expected commit'):
            self.publish()
        self.assertTrue(self.client.release['draft'])
        self.assertFalse(any(method == 'PATCH' for method, _ in self.client.calls))

    def test_stable_replacement_deletes_old_downloads_but_preserves_numeric_tags(self):
        old = self.client.add_release('0.1.2', 'b' * 40)
        self.publish()
        self.assertNotIn(old['id'], self.client.releases)
        self.assertEqual(self.client.tags['0.1.2'], 'b' * 40)
        delete = self.client.events.index(('DELETE', f"/releases/{old['id']}"))
        new_id = self.client.release['id']
        new_urls = {a['browser_download_url'] for a in self.client.asset_lists[new_id]}
        self.assertTrue(new_urls <= {url for kind, url in self.client.events[:delete] if kind == 'download'})
        self.assertEqual(len(self.client.releases), 1)

    def test_failed_new_upload_preserves_previous_stable(self):
        old = self.client.add_release('0.1.2', 'b' * 40)
        self.client.fail_upload_after_save = self.dmg.name
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertIn(old['id'], self.client.releases)
        self.assertEqual(len(self.client.asset_lists[old['id']]), 2)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_stale_stable_version_does_not_mutate_newer_release(self):
        newest = self.client.add_release('0.2.0', 'c' * 40)
        with self.assertRaisesRegex(MODULE.PublishError, 'newer stable'):
            self.publish()
        self.assertIn(newest['id'], self.client.releases)
        self.assertEqual(self.mutations(), [])

    def test_newer_stable_appearing_during_upload_blocks_publication(self):
        previous = self.client.add_release('0.1.2', 'b' * 40)
        def concurrent_publish(method, path, body):
            if method == 'POST' and '/assets?' in path:
                self.client.before_request = None
                self.client.add_release('0.2.0', 'c' * 40)
        self.client.before_request = concurrent_publish
        with self.assertRaisesRegex(MODULE.PublishError, 'newer stable'):
            self.publish()
        self.assertIn(previous['id'], self.client.releases)
        self.assertTrue(self.client.release['draft'])
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_manual_or_foreign_older_release_is_retained(self):
        foreign = self.client.add_release('0.1.0', 'b' * 40)
        foreign['body'] = 'Manually maintained downloads'
        self.publish()
        self.assertIn(foreign['id'], self.client.releases)

    def test_unrecognized_old_attachment_stops_retention(self):
        old = self.client.add_release('0.1.2', 'b' * 40)
        self.client.store_bytes(old['id'], 'manual-notes.txt', b'not managed by Solar')
        with self.assertRaisesRegex(MODULE.PublishError, 'unrecognized attachments'):
            self.publish()
        self.assertIn(old['id'], self.client.releases)

    def test_beta_replaces_verified_release_and_only_moves_beta_tag(self):
        self.client.add_release('0.1.3', self.config.commit)
        old = self.beta()
        self.publish()
        beta = self.client.release
        self.assertFalse(beta['draft'])
        self.assertTrue(beta['prerelease'])
        self.assertEqual(beta['target_commitish'], self.config.commit)
        self.assertEqual(self.client.tags['beta'], self.config.commit)
        self.assertEqual(self.client.tags['0.1.3'], self.config.commit)
        self.assertNotIn(old['id'], self.client.releases)
        self.assertEqual(set(r['tag_name'] for r in self.client.releases.values()), {'0.1.3', 'beta'})
        delete = self.client.events.index(('DELETE', f"/releases/{old['id']}"))
        self.assertGreater(sum(kind == 'download' for kind, _ in self.client.events[:delete]), 1)

    def test_first_beta_can_publish_without_previous_beta(self):
        self.beta(None)
        self.publish()
        self.assertEqual(self.client.release['tag_name'], 'beta')
        self.assertEqual(self.client.tags['beta'], self.config.commit)

    def test_beta_with_failed_stage_upload_keeps_old_downloads_and_tag(self):
        old = self.beta()
        self.client.fail_upload_after_save = self.dmg.name
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertIn(old['id'], self.client.releases)
        self.assertEqual(self.client.tags['beta'], 'b' * 40)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_beta_stale_main_or_changed_previous_release_cannot_replace(self):
        for change in ('head', 'beta'):
            with self.subTest(change=change):
                self.client = FakeGitea(self.config)
                old = self.beta()
                if change == 'head':
                    self.client.head = 'c' * 40
                else:
                    old['target_commitish'] = 'c' * 40
                with self.assertRaises(MODULE.PublishError):
                    self.publish()
                self.assertIn(old['id'], self.client.releases)
                self.assertEqual(self.mutations(), [])

    def test_main_advancing_after_staged_beta_preserves_old_beta(self):
        old = self.beta()
        def move_main(method, path, body):
            if method == 'PATCH' and body == {'draft': False}:
                self.client.head = 'c' * 40
        self.client.before_request = move_main
        with self.assertRaisesRegex(MODULE.PublishError, 'Main advanced'):
            self.publish()
        self.assertIn(old['id'], self.client.releases)
        self.assertEqual(self.client.tags['beta'], 'b' * 40)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_concurrent_beta_replacement_after_staging_is_not_removed(self):
        old = self.beta()
        def replace_beta(method, path, body):
            if method == 'PATCH' and body == {'draft': False}:
                self.client.before_request = None
                old['target_commitish'] = 'c' * 40
                old['body'] = '<!-- solar-beta:v1 commit=' + 'c' * 40 + ' -->'
                self.client.tags['beta'] = 'c' * 40
        self.client.before_request = replace_beta
        with self.assertRaisesRegex(MODULE.PublishError, 'changed since'):
            self.publish()
        self.assertIn(old['id'], self.client.releases)
        self.assertEqual(self.client.tags['beta'], 'c' * 40)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_beta_lost_promotion_response_is_idempotently_reconciled(self):
        self.beta()
        self.client.fail_promote_after_save = True
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertEqual(self.client.release['target_commitish'], self.config.commit)
        self.publish()
        self.assertEqual(len([r for r in self.client.releases.values() if r['tag_name'] == 'beta']), 1)

    def test_same_commit_different_beta_bytes_require_current_baseline(self):
        self.beta(self.config.commit)
        self.publish()
        self.assertEqual(self.client.payload(self.client.stored[0]['browser_download_url']), self.dmg.read_bytes())

    def test_stale_retry_of_same_beta_commit_cannot_overwrite_different_bytes(self):
        self.beta(self.config.commit)
        self.config = replace(self.config, previous_beta='b' * 40)
        self.client.config = self.config
        with self.assertRaisesRegex(MODULE.PublishError, 'stale retry'):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_artifact_retention_only_removes_owned_completed_native_archives(self):
        self.client.add_release('0.1.3', self.config.commit)
        self.client.add_release('beta', 'b' * 40, channel='beta')
        names = ['Solar-0.1.2-macos-arm64', 'Solar-0.1.3-macos-arm64', 'other-file', 'Solar-0.1.0-macos-arm64', 'Solar-0.0.9-macos-arm64']
        for i, name in enumerate(names, 1):
            self.client.artifacts[i] = {'id': i, 'name': name, 'expired': i == 5, 'workflow_run': {'id': i}}
            self.client.runs[i] = {'status': 'completed', 'path': '.gitea/workflows/macos-release.yml@refs/tags/' + name.split('-')[1]}
        self.client.runs[2]['status'] = 'in_progress'
        self.client.runs[4]['path'] = 'unrelated.yml@refs/heads/main'
        MODULE.retain_latest(self.client)
        self.assertNotIn(1, self.client.artifacts)
        self.assertEqual(set(self.client.artifacts), {2, 3, 4, 5})

    def test_artifact_name_must_match_its_actual_workflow_version(self):
        self.client.add_release('0.1.3', self.config.commit)
        self.client.add_release('beta', 'b' * 40, channel='beta')
        self.client.artifacts[1] = {'id': 1, 'name': 'Solar-0.1.2-macos-arm64', 'expired': False,
                                    'workflow_run': {'id': 1}}
        self.client.runs[1] = {'status': 'completed', 'path': 'macos-release.yml@refs/tags/0.1.1'}
        MODULE.retain_latest(self.client)
        self.assertIn(1, self.client.artifacts)

    def test_legacy_manual_archive_requires_exact_workflow_and_matching_source(self):
        self.client.add_release('0.1.3', self.config.commit)
        self.client.add_release('beta', 'b' * 40, channel='beta')
        for i in range(1, 8):
            self.client.artifacts[i] = {'id': i, 'name': 'Solar-macos-arm64', 'expired': False,
                                        'workflow_run': {'id': i, 'head_sha': 'c' * 40}}
            self.client.runs[i] = {'status': 'completed', 'conclusion': 'success', 'event': 'workflow_dispatch',
                                   'path': '.gitea/workflows/macos-release.yml@refs/heads/main', 'head_sha': 'c' * 40}
        self.client.artifacts[2]['name'] = 'Other-macos-arm64'
        self.client.runs[3]['path'] = 'unrelated.yml@refs/heads/main'
        self.client.runs[4]['event'] = 'push'
        self.client.runs[5]['head_sha'] = 'd' * 40
        self.client.artifacts[6]['workflow_run']['head_sha'] = 'not-a-commit'
        self.client.runs[7]['conclusion'] = 'failure'
        MODULE.retain_latest(self.client)
        self.assertEqual(set(self.client.artifacts), set(range(2, 8)))

    def test_legacy_archive_revalidates_source_before_deletion(self):
        self.client.add_release('0.1.3', self.config.commit)
        self.client.add_release('beta', 'b' * 40, channel='beta')
        self.client.artifacts[1] = {'id': 1, 'name': 'Solar-macos-arm64', 'expired': False,
                                    'workflow_run': {'id': 1, 'head_sha': 'c' * 40}}
        self.client.runs[1] = {'status': 'completed', 'conclusion': 'success', 'event': 'workflow_dispatch',
                               'path': 'macos-release.yml@refs/heads/main', 'head_sha': 'c' * 40}
        def change_source(method, path, body):
            if method == 'GET' and path == '/actions/artifacts/1':
                self.client.artifacts[1]['workflow_run']['head_sha'] = 'd' * 40
        self.client.before_request = change_source
        with self.assertRaisesRegex(MODULE.PublishError, 'identity changed'):
            MODULE.retain_latest(self.client)
        self.assertIn(1, self.client.artifacts)

    def test_beta_artifact_requires_matching_source_and_run(self):
        self.client.add_release('0.1.3', self.config.commit)
        self.client.add_release('beta', 'b' * 40, channel='beta')
        for i in range(1, 4):
            self.client.artifacts[i] = {'id': i, 'name': f'Solar-beta-{"b" * 40}-{i}-macos-arm64',
                                        'expired': False, 'workflow_run': {'id': i}}
            self.client.runs[i] = {'status': 'completed', 'path': 'macos-beta.yml@refs/heads/main',
                                   'head_sha': 'b' * 40}
        self.client.runs[2]['head_sha'] = 'c' * 40
        self.client.artifacts[3]['name'] = f'Solar-beta-{"b" * 40}-999-macos-arm64'
        MODULE.retain_latest(self.client)
        self.assertEqual(set(self.client.artifacts), {2, 3})

    def test_retained_download_removed_concurrently_stops_old_release_deletion(self):
        old = self.client.add_release('0.1.2', 'b' * 40)
        current = self.client.add_release('0.1.3', self.config.commit)
        def remove_replacement(method, path, body):
            if method == 'GET' and path == f'/releases/{old["id"]}':
                self.client.before_request = None
                del self.client.releases[current['id']]
                del self.client.asset_lists[current['id']]
        self.client.before_request = remove_replacement
        with self.assertRaisesRegex(MODULE.PublishError, 'stable release changed'):
            MODULE.retain_latest(self.client)
        self.assertIn(old['id'], self.client.releases)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_unknown_beta_tag_prevents_cutover_without_removing_foreign_tag(self):
        self.beta(None)
        self.client.tags['beta'] = 'd' * 40
        with self.assertRaisesRegex(MODULE.PublishError, 'tag changed unexpectedly'):
            self.publish()
        self.assertEqual(self.client.tags['beta'], 'd' * 40)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_unhealthy_beta_blocks_retention_of_old_stable(self):
        old = self.client.add_release('0.1.2', 'b' * 40)
        self.client.add_release('0.1.3', self.config.commit)
        beta = self.client.add_release('beta', 'c' * 40, channel='beta')
        self.client.asset_lists[beta['id']].pop()
        with self.assertRaises(MODULE.PublishError):
            MODULE.retain_latest(self.client)
        self.assertIn(old['id'], self.client.releases)
        self.assertFalse(any(method == 'DELETE' for method, _ in self.client.calls))

    def test_config_rejects_invalid_channels_tags_and_embedded_credentials(self):
        env = {'SOLAR_RELEASE_VERSION':'0.1.3', 'RELEASE_TAG':'0.1.3', 'SOLAR_RELEASE_SHA':'a'*40,
               'SOLAR_RELEASE_DIR':str(self.directory), 'GITEA_SERVER_URL':'https://gitea.example',
               'GITEA_REPOSITORY':'internal/solar-chat', 'GITEA_TOKEN':'dummy-token'}
        self.assertNotIn('dummy-token', repr(MODULE.Config.from_env(env)))
        for update in ({'RELEASE_TAG':'v0.1.3'}, {'SOLAR_BUILD_CHANNEL':'nightly'},
                       {'SOLAR_RELEASE_VERSION':'00.1.3'}, {'SOLAR_RELEASE_SHA':'0'*40},
                       {'GITEA_SERVER_URL':'https://user:password@gitea.example'},
                       {'GITEA_REPOSITORY':'../solar-chat'}, {'GITEA_REPOSITORY':'internal/..'},
                       {'SOLAR_RELEASE_TAG':'0.2.0'}, {'SOLAR_BUILD_CHANNEL':'beta'}):
            with self.subTest(update=update), self.assertRaises(MODULE.PublishError):
                MODULE.Config.from_env({**env, **update})
        beta = {**env, 'SOLAR_BUILD_CHANNEL':'beta', 'RELEASE_TAG':'beta', 'SOLAR_PREVIOUS_BETA_SHA':'b'*40}
        self.assertEqual(MODULE.Config.from_env(beta).tag, 'beta')

    def test_download_redirect_never_forwards_token_to_another_host(self):
        data = self.dmg.read_bytes()
        class Response(io.BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        class Opener:
            def __init__(self): self.calls = []
            def open(self, request, timeout):
                self.calls.append(request)
                if len(self.calls) == 1:
                    raise urllib.error.HTTPError(request.full_url, 302, 'redirect',
                                                 {'Location':'https://storage.example/signed-private-url'}, None)
                return Response(data)
        opener = Opener()
        client = MODULE.Gitea(self.config, opener)
        self.assertEqual(client.download_digest('https://gitea.example/asset/1', len(data)), hashlib.sha256(data).hexdigest())
        self.assertEqual(opener.calls[0].get_header('Authorization'), 'token dummy-token')
        self.assertIsNone(opener.calls[1].get_header('Authorization'))

    def test_changelog_extracts_only_the_exact_version(self):
        path = self.directory / 'CHANGELOG.md'
        path.write_text('# Changelog\n\n## [Unreleased]\nPending.\n\n## [0.1.3] - 2026-11-17\n\n- First release.\n\n## [0.1.2]\nOlder release.\n')
        self.assertEqual(MODULE.notes_from_changelog('0.1.3', path), '- First release.')

    def test_changelog_rejects_missing_empty_and_duplicate_sections(self):
        path = self.directory / 'CHANGELOG.md'
        for text in ('## [0.1.4]\nOther\n', '## [0.1.3]\n\n## [0.1.2]\nOlder\n', '## [0.1.3]\nOne\n## [0.1.3]\nTwo\n'):
            with self.subTest(text=text), self.assertRaises(MODULE.PublishError):
                path.write_text(text)
                MODULE.notes_from_changelog('0.1.3', path)


if __name__ == '__main__':
    unittest.main()
