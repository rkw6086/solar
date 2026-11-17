"""Exercise release ordering and cleanup with fake Apple tools; never signs/uploads."""
import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MOCK = r'''#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
tool, args = pathlib.Path(sys.argv[0]).name, sys.argv[1:]
with open(os.environ['MOCK_CALLS'], 'a') as log:
    log.write(json.dumps([tool, *args]) + '\n')
if tool == 'uname':
    print('Darwin' if args == ['-s'] else os.environ.get('MOCK_ARCH', 'arm64'))
elif tool == 'rustc': print('rustc 1.99.1 (test)')
elif tool == 'node': print('v24.17.0' if '--version' in args else '0.1.0')
elif tool == 'git':
    print('deadbeef' if args == ['rev-parse', 'HEAD'] else '', end='')
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
        print('    "/Users/runner/Library/Keychains/login.keychain-db"')
        print('    "/Library/Keychains/System.keychain"')
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
        assert args[args.index('-p')+1] == 'codeSign' and '-L' in args
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
elif tool == 'PlistBuddy': print('0.1.0')
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
        self.env = {
            'PATH': f'{self.bin}:{os.environ["PATH"]}',
            'RUNNER_TEMP': str(self.root), 'CARGO_TARGET_DIR': str(self.root / 'target'),
            'SOLAR_RELEASE_DIR': str(self.root / 'artifacts'), 'MOCK_CALLS': str(self.calls),
            'MOCK_EFFECTIVE_LIST': str(self.root / 'effective-list.json'),
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

    def recorded(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def assert_cleaned(self):
        self.assertEqual(list(self.root.glob('solar-sign.*')), [])
        calls = self.recorded()
        lists = [call for call in calls if call[:2] == ['security', 'list-keychains'] and '-s' in call]
        if lists:
            self.assertEqual(lists[-1][-2:], ['/Users/runner/Library/Keychains/login.keychain-db', '/Library/Keychains/System.keychain'])

    def test_build_and_package_orders_signatures_and_staples_both(self):
        result = self.run_script('build-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.run_script('package-macos.sh')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_cleaned()
        calls = self.recorded()
        intermediate = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'add-certificates'])
        identity = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'import'])
        trust = next(i for i, call in enumerate(calls) if call[:2] == ['security', 'verify-cert'])
        first_signature = next(i for i, call in enumerate(calls) if call[0] == 'codesign' and '--sign' in call)
        self.assertLess(intermediate, identity)
        self.assertLess(identity, trust)
        self.assertLess(trust, first_signature)
        self.assertFalse(any(call[:2] == ['security', 'add-trusted-cert'] for call in calls))
        signatures = [call[-1] for call in calls if call[0] == 'codesign' and '--sign' in call]
        self.assertEqual([Path(path).name for path in signatures], ['solar-native-helper', 'solar', 'Solar.app', 'Solar_0.1.0_aarch64.dmg'])
        notarized = [call[3] for call in calls if call[:3] == ['xcrun', 'notarytool', 'submit']]
        self.assertEqual([Path(path).suffix for path in notarized], ['.zip', '.dmg'])
        staples = [call[-1] for call in calls if call[:3] == ['xcrun', 'stapler', 'staple']]
        self.assertEqual([Path(path).suffix for path in staples], ['.app', '.dmg'])
        self.assertTrue((self.root / 'artifacts/SHA256SUMS').is_file())
        for secret in ('dummy-p12-password', self.env['APPLE_CERTIFICATE'], self.env['APPLE_API_KEY_P8']):
            self.assertNotIn(secret, result.stdout + result.stderr)

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


if __name__ == '__main__':
    unittest.main()
