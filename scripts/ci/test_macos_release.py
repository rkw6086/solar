"""Exercise release ordering and cleanup with fake Apple tools; never signs/uploads."""
import base64
import json
import os
import plistlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MOCK = r'''#!/usr/bin/env python3
import json, os, pathlib, plistlib, shutil, sys
tool, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ['MOCK_CALLS'], 'a') as log:
    log.write(json.dumps([tool, *args]) + '\n')
if tool == 'uname':
    print('Darwin' if args == ['-s'] else os.environ.get('MOCK_ARCH', 'arm64'))
elif tool == 'rustc': print('rustc 1.99.1 (test)')
elif tool == 'node': print('v24.17.0' if '--version' in args else '0.1.0')
elif tool == 'git':
    print(os.environ['SOLAR_RELEASE_SHA'] if args == ['rev-parse', 'HEAD'] else '', end='')
elif tool == 'openssl':
    if args[0] == 'rand': print('a' * 64)
    elif args[0] == 'x509':
        assert b'BEGIN CERTIFICATE' in pathlib.Path(args[args.index('-in')+1]).read_bytes()
        assert args[args.index('-outform')+1] == 'DER'
        pathlib.Path(args[args.index('-out')+1]).write_bytes(b'dummy-intermediate-der')
elif tool == 'security':
    action = args[0]
    if action == 'list-keychains' and '-s' not in args:
        if os.environ.get('MOCK_KEYCHAIN_LIST_FAIL'): sys.exit(1)
        if '-d' in args and os.environ.get('MOCK_USER_KEYCHAIN_LIST_FAIL'): sys.exit(1)
        print('    "/Users/runner/Library/Keychains/login.keychain-db"')
        if '-d' not in args: print('    "/Library/Keychains/System.keychain"')
    elif action == 'list-keychains' and '-s' in args:
        # CI can have an effective search list distinct from saved user preferences.
        if '-d' not in args:
            pathlib.Path(os.environ['MOCK_EFFECTIVE_LIST']).write_text(json.dumps(args[args.index('-s')+1:]))
    elif action == 'create-keychain': pathlib.Path(args[-1]).touch()
    elif action == 'delete-keychain': pathlib.Path(args[-1]).unlink()
    elif action == 'find-identity': print('1) 1234 "Developer ID Application: Solar Test (ABCDEFGHIJ)"')
    elif action == 'add-certificates':
        if os.environ.get('MOCK_INTERMEDIATE_FAIL'): sys.exit(1)
        assert pathlib.Path(args[-1]).name == 'DeveloperIDG2CA.cer'
        assert pathlib.Path(args[-1]).read_bytes() == b'dummy-intermediate-der'
        pathlib.Path(args[args.index('-k')+1] + '.intermediate').touch()
    elif action == 'import':
        assert pathlib.Path(args[1]).read_bytes() == b'dummy-p12'
    elif action == 'find-certificate': print('dummy-public-leaf')
    elif action == 'verify-cert':
        assert args[args.index('-p')+1] in ('basic', 'codeSign') and '-L' in args
        assert '-r' not in args
        assert pathlib.Path(args[args.index('-c')+1]).read_text().strip() == 'dummy-public-leaf'
        search = json.loads(pathlib.Path(os.environ['MOCK_EFFECTIVE_LIST']).read_text())
        assert any(pathlib.Path(path + '.intermediate').exists() for path in search)
        assert '-k' not in args
        if os.environ.get('MOCK_CHAIN_FAIL'): sys.exit(1)
elif tool == 'codesign':
    if os.environ.get('MOCK_SIGN_FAIL'): sys.exit(1)
    if '--sign' in args:
        assert pathlib.Path(args[args.index('--keychain')+1] + '.intermediate').exists()
        search = json.loads(pathlib.Path(os.environ['MOCK_EFFECTIVE_LIST']).read_text())
        assert args[args.index('--keychain')+1] in search
    if '-dv' in args:
        print('TeamIdentifier=ABCDEFGHIJ\nflags=0x10000(runtime)', file=sys.stderr)
elif tool == 'PlistBuddy':
    field = args[1].removeprefix('Print :')
    with open(args[-1], 'rb') as source: data = plistlib.load(source)
    override = {'CFBundleShortVersionString': 'MOCK_BUNDLE_VERSION', 'SolarBuildChannel': 'MOCK_BUNDLE_CHANNEL', 'SolarSourceCommit': 'MOCK_BUNDLE_COMMIT'}.get(field)
    print(os.environ.get(override, data[field]) if override else data[field])
elif tool == 'lipo':
    assert len(args) == 3 and pathlib.Path(args[0]).is_file()
    assert args[1:] == ['-verify_arch', 'arm64']
elif tool == 'ditto':
    if '-c' in args: pathlib.Path(args[-1]).write_bytes(b'dummy-zip')
    else: shutil.copytree(args[-2], args[-1])
elif tool == 'hdiutil':
    if args[0] == 'create':
        root = pathlib.Path(args[args.index('-srcfolder')+1])
        assert (root / 'Solar.app').is_dir()
        assert os.readlink(root / 'Applications') == '/Applications'
        pathlib.Path(args[-1]).write_bytes(b'dummy-dmg')
elif tool == 'xcrun':
    if args[:2] == ['notarytool', 'submit']:
        assert pathlib.Path(args[args.index('--key')+1]).read_bytes() == b'dummy-p8'
        print(json.dumps({'id':'test-submission','status':'Invalid' if os.environ.get('MOCK_NOTARY_REJECT') else 'Accepted'}))
    elif args[:2] == ['notarytool', 'log']:
        pathlib.Path(args[-1]).write_text('{"issues":[]}')
elif tool == 'corepack' and 'tauri' in args:
    app = pathlib.Path(os.environ['CARGO_TARGET_DIR']) / 'aarch64-apple-darwin/release/bundle/macos/Solar.app/Contents/MacOS'
    app.mkdir(parents=True)
    for name in ('solar', 'solar-native-helper'):
        path = app / name
        path.write_text('dummy executable')
        path.chmod(0o755)
    with (app.parent / 'Info.plist').open('wb') as destination:
        plistlib.dump({'CFBundleShortVersionString':'0.1.0'}, destination)
'''


class ReleaseTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        mock = self.bin / 'mock'
        mock.write_text(MOCK)
        mock.chmod(0o755)
        for name in ('uname', 'rustc', 'node', 'git', 'openssl', 'security', 'codesign',
                     'PlistBuddy', 'ditto', 'hdiutil', 'xcrun', 'corepack', 'cargo', 'lipo', 'xcode-select', 'spctl'):
            (self.bin / name).symlink_to(mock)
        self.calls = self.root / 'calls.jsonl'
        self.dmg_failure = self.root / 'dmg-failure'
        dmg_python = self.bin / 'dmg-python'
        # The real helper runs with env -i. Embed fixture paths so this mock
        # cannot accidentally depend on (or permit) inherited build secrets.
        dmg_python.write_text(f'''#!{sys.executable}
import json, os, pathlib, sys
args = sys.argv[1:]
assert args[:2] == ['-I', 'scripts/ci/create-dmg.py']
args = args[2:]
with pathlib.Path({str(self.calls)!r}).open('a') as log:
    log.write(json.dumps(['create-dmg.py', *args]) + '\\n')
assert set(os.environ) <= {{'PATH', 'HOME', 'TMPDIR', 'LC_CTYPE'}}, 'Build environment leaked to layout helper'
assert not any(name.startswith('APPLE_') for name in os.environ)
assert os.environ['PATH'] == '/usr/bin:/bin:/usr/sbin:/sbin'
assert '--verify-signature' in args
assert pathlib.Path(args[args.index('--app') + 1]).is_dir()
output = pathlib.Path(args[args.index('--output') + 1])
phase = 'verify' if '--verify-only' in args else 'create'
if phase == 'create':
    assert not output.exists()
    output.write_bytes(b'dummy-dmg')
else:
    assert output.read_bytes() == b'dummy-dmg'
failure = pathlib.Path({str(self.dmg_failure)!r})
if failure.exists() and failure.read_text() == phase:
    raise SystemExit('Simulated DMG ' + phase + ' failure')
''')
        dmg_python.chmod(0o755)
        home = self.root / 'home'
        (home / 'Library/Keychains').mkdir(parents=True)
        self.env = {
            'HOME': str(home),
            'PATH': f'{self.bin}:{os.environ["PATH"]}',
            'RUNNER_TEMP': str(self.root), 'CARGO_TARGET_DIR': str(self.root / 'target'),
            'SOLAR_DMG_PYTHON': str(dmg_python),
            'SOLAR_RELEASE_DIR': str(self.root / 'artifacts'), 'MOCK_CALLS': str(self.calls),
            'MOCK_EFFECTIVE_LIST': str(self.root / 'effective-list.json'),
            'GITHUB_EVENT_NAME': 'push', 'GITHUB_REF': 'refs/tags/0.1.0',
            'SOLAR_RELEASE_VERSION': '0.1.0',
            'SOLAR_RELEASE_SHA': 'a' * 40,
            'APPLE_CERTIFICATE': base64.b64encode(b'dummy-p12').decode(),
            'APPLE_CERTIFICATE_PASSWORD': 'dummy-p12-password',
            'APPLE_SIGNING_IDENTITY': 'Developer ID Application: Solar Test (ABCDEFGHIJ)',
            'APPLE_TEAM_ID': 'ABCDEFGHIJ', 'APPLE_API_KEY': 'TESTKEY123',
            'APPLE_API_ISSUER': '00000000-1111-2222-3333-444444444444',
            'APPLE_API_KEY_P8': base64.b64encode(b'dummy-p8').decode(),
        }
        self.app = Path(self.env['CARGO_TARGET_DIR']) / 'aarch64-apple-darwin/release/bundle/macos/Solar.app'

    def tearDown(self):
        self.temp.cleanup()

    def run_script(self, filename):
        text = (ROOT / 'scripts/ci' / filename).read_text()
        # Absolute Apple tool paths are fixed in production; substitute only in this test copy.
        for tool in ('security', 'codesign', 'openssl', 'PlistBuddy', 'spctl'):
            for prefix in ('/usr/bin/', '/usr/sbin/', '/usr/libexec/'):
                text = text.replace(prefix + tool, str(self.bin / tool))
        script = self.root / filename
        script.write_text(text)
        return subprocess.run(['bash', str(script)], env=self.env, cwd=ROOT, text=True, capture_output=True)

    def prepare_app(self):
        for name in ('solar', 'solar-native-helper'):
            path = self.app / 'Contents/MacOS' / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('dummy')
            path.chmod(0o755)
        with (self.app / 'Contents/Info.plist').open('wb') as destination:
            plistlib.dump({'CFBundleShortVersionString': '0.1.0',
                           'SolarBuildChannel': self.env.get('SOLAR_BUILD_CHANNEL', 'release'),
                           'SolarSourceCommit': self.env['SOLAR_RELEASE_SHA']}, destination)

    def recorded(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def assert_cleaned(self):
        self.assertEqual(list(self.root.rglob('solar-sign.*')), [])
        calls = self.recorded()
        lists = [call for call in calls if call[:2] == ['security', 'list-keychains'] and '-s' in call]
        if lists:
            self.assertEqual(lists[-2], ['security', 'list-keychains', '-s', '/Users/runner/Library/Keychains/login.keychain-db', '/Library/Keychains/System.keychain'])
            self.assertEqual(lists[-1], ['security', 'list-keychains', '-d', 'user', '-s', '/Users/runner/Library/Keychains/login.keychain-db'])

    def test_build_and_package_orders_signatures_and_staples_both(self):
        result = self.run_script('build-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.run_script('package-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_cleaned()
        calls = self.recorded()
        intermediate = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'add-certificates'])
        search_list = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'list-keychains'] and '-s' in call)
        identity = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'import'])
        trust = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'verify-cert'])
        first_signature = next(i for i, call in enumerate(calls) if call[0] == 'codesign' and '--sign' in call)
        self.assertLess(search_list, intermediate)
        self.assertLess(intermediate, identity)
        self.assertLess(identity, trust)
        self.assertLess(trust, first_signature)
        self.assertFalse(any(call[:2] == ['security', 'add-trusted-cert'] for call in calls))
        policies = [call[call.index('-p')+1] for call in calls if call[:2] == ['security', 'verify-cert']]
        self.assertEqual(policies, ['basic', 'codeSign'])
        signatures = [call[-1] for call in calls if call[0] == 'codesign' and '--sign' in call]
        self.assertEqual([Path(path).name for path in signatures], ['solar-native-helper', 'solar', 'Solar.app', 'Solar_0.1.0_aarch64.dmg'])
        notarized = [call[3] for call in calls if call[:3] == ['xcrun', 'notarytool', 'submit']]
        self.assertEqual([Path(path).suffix for path in notarized], ['.zip', '.dmg'])
        staples = [call[-1] for call in calls if call[:3] == ['xcrun', 'stapler', 'staple']]
        self.assertEqual([Path(path).suffix for path in staples], ['.app', '.dmg'])
        layout_calls = [(i, call) for i, call in enumerate(calls) if call[0] == 'create-dmg.py']
        self.assertEqual(len(layout_calls), 2)
        creation, verification = layout_calls
        self.assertNotIn('--verify-only', creation[1])
        self.assertIn('--verify-only', verification[1])
        self.assertEqual(creation[1][creation[1].index('--volume-name') + 1], 'Solar')
        app_staple = next(i for i, call in enumerate(calls)
                          if call[:3] == ['xcrun', 'stapler', 'staple'] and call[-1].endswith('.app'))
        dmg_signature = next(i for i, call in enumerate(calls)
                             if call[0] == 'codesign' and '--sign' in call and call[-1].endswith('.dmg'))
        dmg_staple = next(i for i, call in enumerate(calls)
                         if call[:3] == ['xcrun', 'stapler', 'staple'] and call[-1].endswith('.dmg'))
        dmg_integrity = next(i for i, call in enumerate(calls) if call[:2] == ['hdiutil', 'verify'])
        self.assertLess(app_staple, creation[0])
        self.assertLess(creation[0], dmg_signature)
        self.assertLess(dmg_staple, verification[0])
        self.assertLess(dmg_integrity, verification[0])
        self.assertFalse(any(call[:2] == ['hdiutil', 'create'] for call in calls))
        self.assertTrue((self.root / 'artifacts/SHA256SUMS').is_file())
        identity = json.loads((self.root / 'artifacts/BUILD-INFO.json').read_text())
        self.assertEqual(identity, {'version': '0.1.0', 'source': 'a' * 40,
                                   'channel': 'release', 'target': 'aarch64-apple-darwin',
                                   'signed': True, 'packages': ['Solar_0.1.0_aarch64.dmg']})
        for secret in ('dummy-p12-password', self.env['APPLE_CERTIFICATE'], self.env['APPLE_API_KEY_P8']):
            self.assertNotIn(secret, result.stdout + result.stderr)

    def beta_environment(self, event='schedule'):
        self.env.update(SOLAR_BUILD_CHANNEL='beta', SOLAR_BETA_ID=self.env['SOLAR_RELEASE_SHA'],
                        GITHUB_EVENT_NAME=event, GITHUB_REF='refs/heads/main')

    def test_beta_build_embeds_channel_and_source_identity(self):
        self.beta_environment()
        result = self.run_script('build-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.app / 'Contents/Info.plist').open('rb') as source:
            data = plistlib.load(source)
        self.assertEqual(data['SolarBuildChannel'], 'beta')
        self.assertEqual(data['SolarSourceCommit'], self.env['SOLAR_RELEASE_SHA'])
        result = self.run_script('package-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / f'artifacts/Solar_beta_{"a" * 40}_aarch64.dmg').is_file())
        self.assert_cleaned()

    def test_github_pull_request_builds_without_signing(self):
        self.env.update(SOLAR_BUILD_CHANNEL='ci', SOLAR_GITHUB_CI='1',
                        GITHUB_SERVER_URL='https://github.com', GITHUB_REPOSITORY='rkw6086/solar',
                        GITHUB_EVENT_NAME='pull_request', GITHUB_REF='refs/pull/42/merge')
        result = self.run_script('build-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        with (self.app / 'Contents/Info.plist').open('rb') as source:
            self.assertEqual(plistlib.load(source)['SolarBuildChannel'], 'ci')
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(call[0] in ('security', 'codesign') for call in self.recorded()))

    def test_foreign_repository_cannot_request_unsigned_build_mode(self):
        self.env.update(SOLAR_BUILD_CHANNEL='ci', SOLAR_GITHUB_CI='1',
                        GITHUB_SERVER_URL='https://github.com', GITHUB_REPOSITORY='someone/solar',
                        GITHUB_EVENT_NAME='pull_request')
        result = self.run_script('build-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(call[0] == 'corepack' for call in self.recorded()))

    def test_manual_beta_can_sign_main(self):
        self.beta_environment('workflow_dispatch')
        self.prepare_app()
        result = self.run_script('package-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_cleaned()

    def test_beta_rejects_branch_push_wrong_branch_and_wrong_identity_before_keys(self):
        self.beta_environment()
        self.prepare_app()
        for updates in ({'GITHUB_EVENT_NAME': 'push'}, {'GITHUB_REF': 'refs/heads/feature'},
                        {'SOLAR_BETA_ID': 'b' * 40}):
            previous = self.env.copy()
            self.env.update(updates)
            self.assertNotEqual(self.run_script('build-macos.sh').returncode, 0)
            self.assertNotEqual(self.run_script('package-macos.sh').returncode, 0)
            self.env = previous
        self.assertFalse(any(call[0] == 'security' for call in self.recorded()))

    def test_bundle_channel_and_commit_must_match_before_keys(self):
        self.prepare_app()
        for key, value in [('MOCK_BUNDLE_CHANNEL', 'beta'), ('MOCK_BUNDLE_COMMIT', 'b' * 40)]:
            self.env[key] = value
            result = self.run_script('package-macos.sh')
            self.assertNotEqual(result.returncode, 0)
            del self.env[key]
        self.assertFalse(any(call[0] == 'security' for call in self.recorded()))

    def test_notary_rejection_cleans_credentials_and_distributable(self):
        self.prepare_app()
        self.env['MOCK_NOTARY_REJECT'] = '1'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / 'artifacts').exists())
        self.assert_cleaned()

    def test_signing_failure_cleans_credentials(self):
        self.prepare_app()
        self.env['MOCK_SIGN_FAIL'] = '1'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assert_cleaned()

    def test_dmg_creation_failure_cleans_credentials_and_partial_image(self):
        self.prepare_app()
        self.dmg_failure.write_text('create')
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Simulated DMG create failure', result.stderr)
        self.assertFalse((self.root / 'artifacts').exists())
        self.assert_cleaned()
        calls = self.recorded()
        self.assertTrue(any(call[:2] == ['security', 'delete-keychain'] for call in calls))
        self.assertFalse(any(call[0] == 'codesign' and '--sign' in call and call[-1].endswith('.dmg')
                             for call in calls))

    def test_final_dmg_layout_failure_prevents_publication(self):
        self.prepare_app()
        self.dmg_failure.write_text('verify')
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Simulated DMG verify failure', result.stderr)
        self.assertNotIn('Verified signed and notarized DMG:', result.stdout)
        self.assertFalse((self.root / 'artifacts').exists())
        self.assert_cleaned()
        calls = self.recorded()
        self.assertTrue(any(call[:2] == ['security', 'delete-keychain'] for call in calls))
        self.assertTrue(any(call[:3] == ['xcrun', 'stapler', 'staple'] and call[-1].endswith('.dmg')
                            for call in calls))

    def test_missing_dmg_interpreter_stops_before_importing_keys(self):
        self.prepare_app()
        self.env.pop('SOLAR_DMG_PYTHON')
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Prepare the pinned DMG layout tools before signing', result.stderr)
        self.assertFalse(any(call[0] == 'security' for call in self.recorded()))

    def test_intermediate_import_failure_cleans_credentials_before_signing(self):
        self.prepare_app()
        self.env['MOCK_INTERMEDIATE_FAIL'] = '1'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assert_cleaned()
        self.assertFalse(any(call[0] == 'codesign' for call in self.recorded()))

    def test_untrusted_chain_stops_before_signing_or_notarization(self):
        self.prepare_app()
        self.env['MOCK_CHAIN_FAIL'] = '1'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('certificate chain is not trusted', result.stderr)
        self.assert_cleaned()
        self.assertFalse(any(call[0] == 'codesign' for call in self.recorded()))
        self.assertFalse(any(call[:2] == ['xcrun', 'notarytool'] for call in self.recorded()))

    def test_keychain_list_failure_never_changes_search_list(self):
        self.prepare_app()
        self.env['MOCK_KEYCHAIN_LIST_FAIL'] = '1'
        self.assertNotEqual(self.run_script('package-macos.sh').returncode, 0)
        self.assert_cleaned()
        self.assertFalse(any('-s' in call for call in self.recorded() if call[:2] == ['security', 'list-keychains']))

    def test_user_keychain_list_failure_never_changes_search_list(self):
        self.prepare_app()
        self.env['MOCK_USER_KEYCHAIN_LIST_FAIL'] = '1'
        self.assertNotEqual(self.run_script('package-macos.sh').returncode, 0)
        self.assert_cleaned()
        self.assertFalse(any('-s' in call for call in self.recorded() if call[:2] == ['security', 'list-keychains']))

    def test_build_rejects_wrong_tag_or_architecture(self):
        self.env['GITHUB_REF'] = 'refs/tags/v999.0.0'
        self.assertNotEqual(self.run_script('build-macos.sh').returncode, 0)
        self.env.pop('GITHUB_REF')
        self.env['MOCK_ARCH'] = 'x86_64'
        self.assertNotEqual(self.run_script('build-macos.sh').returncode, 0)

    def test_package_rejects_missing_native_helper_before_importing_keys(self):
        self.prepare_app()
        (self.app / 'Contents/MacOS/solar-native-helper').unlink()
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('security', [call[0] for call in self.recorded()])

    def test_package_rejects_wrong_certificate_team_before_importing_keys(self):
        self.prepare_app()
        self.env['APPLE_TEAM_ID'] = 'WRONGTEAM1'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('security', [call[0] for call in self.recorded()])

    def test_branch_and_manual_invocations_cannot_sign(self):
        self.prepare_app()
        for event, ref in [('push', 'refs/heads/main'), ('workflow_dispatch', 'refs/tags/0.1.0')]:
            self.env['GITHUB_EVENT_NAME'] = event
            self.env['GITHUB_REF'] = ref
            result = self.run_script('package-macos.sh')
            self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(call[0] == 'security' for call in self.recorded()))

    def test_bundle_version_must_match_tag_before_importing_keys(self):
        self.prepare_app()
        self.env['MOCK_BUNDLE_VERSION'] = '0.2.0'
        result = self.run_script('package-macos.sh')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('bundle version', result.stderr)
        self.assertFalse(any(call[0] == 'security' for call in self.recorded()))


if __name__ == '__main__':
    unittest.main()
