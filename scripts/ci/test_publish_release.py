"""Release publication behavior without a network connection or real credentials."""
import copy
from dataclasses import replace
import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
import urllib.error

SPEC = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish-release.py"))
MODULE = importlib.util.module_from_spec(SPEC)
import sys
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class FakeGitea:
    def __init__(self, config):
        self.config = config
        self.release = None
        self.stored = []
        self.calls = []
        self.tag_reads = 0
        self.move_tag = False
        self.fail_upload_after_save = None

    def generated_release(self, draft=True):
        return {"id": 10, "draft": draft, "prerelease": False,
                "tag_name": self.config.tag, "target_commitish": self.config.commit,
                "body": f"<!-- solar-release:v1 tag={self.config.tag} commit={self.config.commit} -->"}

    def store(self, path, data=None):
        payload = path.read_bytes() if data is None else data
        asset = {"id": len(self.stored) + 1, "name": path.name, "size": len(payload),
                 "browser_download_url": "https://gitea.example/asset/" + str(len(self.stored) + 1),
                 "payload": payload}
        self.stored.append(asset)

    def request(self, method, path, body=None, upload=None, missing_ok=False):
        self.calls.append((method, path))
        if method == "GET" and path.startswith("/tags/"):
            self.tag_reads += 1
            sha = "b" * 40 if self.move_tag and self.tag_reads > 1 else self.config.commit
            return {"name": self.config.tag, "commit": {"sha": sha}}
        if method == "GET" and path.startswith("/releases/tags/"):
            return copy.deepcopy(self.release)
        if method == "POST" and path == "/releases":
            self.release = {"id": 10, **copy.deepcopy(body)}
            return copy.deepcopy(self.release)
        if method == "GET" and path.endswith("/assets"):
            return [{k: v for k, v in a.items() if k != "payload"} for a in self.stored]
        if method == "POST" and "/assets?" in path:
            self.store(upload)
            if self.fail_upload_after_save == upload.name:
                self.fail_upload_after_save = None
                raise MODULE.PublishError("Simulated connection loss after upload.")
            return {k: v for k, v in self.stored[-1].items() if k != "payload"}
        if method == "GET" and path == "/releases/10":
            return copy.deepcopy(self.release)
        if method == "PATCH" and path == "/releases/10":
            self.release.update(body)
            return copy.deepcopy(self.release)
        raise AssertionError((method, path))

    def download_digest(self, url, expected_size):
        asset = next(a for a in self.stored if a["browser_download_url"] == url)
        assert expected_size == len(asset["payload"])
        return hashlib.sha256(asset["payload"]).hexdigest()


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.directory = Path(self.temp.name)
        self.config = MODULE.Config("0.1.0", "0.1.0", self.directory,
                                    "https://gitea.example", "internal/solar-chat", "a" * 40, "dummy-token",
                                    notes="- Initial signed macOS release.")
        self.dmg = self.directory / "Solar_0.1.0_aarch64.dmg"
        self.dmg.write_bytes(b"signed-and-notarized-test-image")
        self.sums = self.directory / "SHA256SUMS"
        self.sums.write_text(hashlib.sha256(self.dmg.read_bytes()).hexdigest() + "  " + self.dmg.name + "\n")
        self.client = FakeGitea(self.config)

    def tearDown(self):
        self.temp.cleanup()

    def publish(self):
        return MODULE.publish(self.config, self.client)

    def mutations(self):
        return [call for call in self.client.calls if call[0] != "GET"]

    def test_creates_draft_verifies_both_assets_then_publishes(self):
        url = self.publish()
        self.assertEqual(url, "https://gitea.example/internal/solar-chat/releases/tag/0.1.0")
        self.assertFalse(self.client.release["draft"])
        self.assertIn("- Initial signed macOS release.", self.client.release["body"])
        self.assertEqual([a["name"] for a in self.client.stored], [self.dmg.name, "SHA256SUMS"])
        self.assertEqual([call[0] for call in self.mutations()], ["POST", "POST", "POST", "PATCH"])
        self.assertEqual(self.client.calls[-1], ("PATCH", "/releases/10"))

    def test_bad_local_checksum_never_calls_server(self):
        self.sums.write_text("0" * 64 + "  " + self.dmg.name + "\n")
        with self.assertRaisesRegex(MODULE.PublishError, "SHA256SUMS"):
            self.publish()
        self.assertEqual(self.client.calls, [])

    def test_empty_release_notes_never_call_server(self):
        with self.assertRaisesRegex(MODULE.PublishError, "notes"):
            MODULE.publish(replace(self.config, notes="  "), self.client)
        self.assertEqual(self.client.calls, [])

    def test_rejects_symlink_asset(self):
        original = self.directory / "other.dmg"
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
        self.assertEqual(sum("/assets?" in path for _, path in self.mutations()), 1)

    def test_lost_upload_response_resumes_existing_bytes(self):
        self.client.fail_upload_after_save = self.sums.name
        with self.assertRaises(MODULE.PublishError):
            self.publish()
        self.assertTrue(self.client.release["draft"])
        self.assertEqual(len(self.client.stored), 2)
        self.publish()
        self.assertFalse(self.client.release["draft"])
        self.assertEqual(len(self.client.stored), 2)

    def test_foreign_draft_remains_untouched(self):
        self.client.release = self.client.generated_release()
        self.client.release["body"] = "Manually prepared release"
        with self.assertRaisesRegex(MODULE.PublishError, "left unchanged"):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_mismatching_asset_is_not_replaced(self):
        self.client.release = self.client.generated_release()
        self.client.store(self.dmg, b"x" * self.dmg.stat().st_size)
        with self.assertRaisesRegex(MODULE.PublishError, "checksum differs"):
            self.publish()
        self.assertEqual(self.mutations(), [])
        self.assertTrue(self.client.release["draft"])

    def test_duplicate_assets_remain_unpublished(self):
        self.client.release = self.client.generated_release()
        self.client.store(self.dmg)
        self.client.store(self.dmg)
        with self.assertRaisesRegex(MODULE.PublishError, "duplicate"):
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
        with self.assertRaisesRegex(MODULE.PublishError, "incomplete"):
            self.publish()
        self.assertEqual(self.mutations(), [])

    def test_tag_moved_during_upload_leaves_draft(self):
        self.client.move_tag = True
        with self.assertRaisesRegex(MODULE.PublishError, "expected commit"):
            self.publish()
        self.assertTrue(self.client.release["draft"])
        self.assertFalse(any(method == "PATCH" for method, _ in self.client.calls))

    def test_config_rejects_nonversion_tags_and_embedded_credentials(self):
        env = {"SOLAR_RELEASE_VERSION": "0.1.0", "RELEASE_TAG": "0.1.0",
               "SOLAR_RELEASE_SHA": "a" * 40, "SOLAR_RELEASE_DIR": str(self.directory),
               "GITEA_SERVER_URL": "https://gitea.example", "GITEA_REPOSITORY": "internal/solar-chat",
               "GITEA_TOKEN": "dummy-token"}
        self.assertNotIn("dummy-token", repr(MODULE.Config.from_env(env)))
        for update in ({"RELEASE_TAG": "v0.1.0"}, {"SOLAR_RELEASE_VERSION": "00.1.0"},
                       {"GITEA_SERVER_URL": "https://user:password@gitea.example"},
                       {"GITEA_REPOSITORY": "../solar-chat"}, {"GITEA_REPOSITORY": "internal/.."},
                       {"SOLAR_RELEASE_TAG": "0.2.0"}, {"SOLAR_RELEASE_SHA": "0" * 40}):
            with self.subTest(update=update), self.assertRaises(MODULE.PublishError):
                MODULE.Config.from_env({**env, **update})

    def test_download_redirect_never_forwards_token_to_another_host(self):
        data = self.dmg.read_bytes()

        class Response(io.BytesIO):
            def __enter__(self):
                return self

            def __exit__(self, *args):
                self.close()

        class Opener:
            calls = []

            def open(self, request, timeout):
                self.calls.append(request)
                if len(self.calls) == 1:
                    raise urllib.error.HTTPError(request.full_url, 302, "redirect",
                                                 {"Location": "https://storage.example/signed-private-url"}, None)
                return Response(data)

        opener = Opener()
        client = MODULE.Gitea(self.config, opener)
        digest = client.download_digest("https://gitea.example/asset/1", len(data))
        self.assertEqual(digest, hashlib.sha256(data).hexdigest())
        self.assertEqual(opener.calls[0].get_header("Authorization"), "token dummy-token")
        self.assertIsNone(opener.calls[1].get_header("Authorization"))

    def test_changelog_extracts_only_the_exact_version(self):
        path = self.directory / "CHANGELOG.md"
        path.write_text("# Changelog\n\n## [Unreleased]\nPending.\n\n"
                        "## [0.1.0] - 2026-11-17\n\n- First release.\n\n"
                        "## [0.0.9]\nOlder release.\n")
        self.assertEqual(MODULE.notes_from_changelog("0.1.0", path), "- First release.")

    def test_changelog_rejects_missing_empty_and_duplicate_sections(self):
        path = self.directory / "CHANGELOG.md"
        for text in ("## [0.1.1]\nOther\n", "## [0.1.0]\n\n## [0.0.9]\nOlder\n",
                     "## [0.1.0]\nOne\n## [0.1.0]\nTwo\n"):
            with self.subTest(text=text), self.assertRaises(MODULE.PublishError):
                path.write_text(text)
                MODULE.notes_from_changelog("0.1.0", path)


if __name__ == "__main__":
    unittest.main()
