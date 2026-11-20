"""Repository synchronization policy without live credentials or remote writes."""
import contextlib
import importlib.util
import io
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("solar_repository_sync", Path(__file__).with_name("sync-repositories.py"))
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)

MAIN = "refs/heads/main"
A, B, C = "a" * 40, "b" * 40, "c" * 40


class FakeRepository:
    peer = "github"
    remotes = {"gitea": "primary", "github": "peer"}

    def __init__(self, primary, peer, ancestry=(), merge_result=True):
        self.snapshots = {"gitea": primary, "github": peer}
        self.ancestry = set(ancestry)
        self.merge_result = merge_result
        self.transfers = []
        self.fetches = []
        self.merges = []

    def refs(self, remote):
        return dict(self.snapshots[remote])

    def fetch(self, remote, refs):
        self.fetches.append((remote, dict(refs)))

    def ancestor(self, older, newer):
        return (older, newer) in self.ancestry

    def merge(self, ref, primary, peer, dry_run):
        self.merges.append((ref, primary, peer, dry_run))
        return self.merge_result

    def push(self, source, destination, ref, oid, expected, dry_run):
        self.transfers.append((source, destination, ref, oid, expected, dry_run))


class TransferHarness(sync.Repository):
    def __init__(self, states, token="temporary-test-token"):
        self.peer = "github"
        self.remotes = {"gitea": "primary", "github": "peer"}
        self.env = {"GITEA_TOKEN": token} if token else {}
        self.states = iter(states)
        self.operations = []

    def refs(self, remote, ref):
        value = next(self.states)
        return {} if value is None else {ref: value}

    def fetch_lfs(self, remote, oid):
        self.operations.append(("fetch-lfs", remote, oid))

    def push_lfs(self, remote, oid):
        self.operations.append(("push-lfs", remote, oid))

    def git(self, *args, **kwargs):
        self.operations.append(("git", args, kwargs))
        return 0, ""


class RepositorySyncTests(unittest.TestCase):
    def run_sync(self, repository, dry_run=False):
        with contextlib.redirect_stdout(io.StringIO()):
            return sync.synchronize(repository, dry_run)

    def test_matching_refs_skip_git_objects_and_lfs(self):
        repository = FakeRepository({MAIN: A, "refs/tags/0.1.3": B}, {MAIN: A, "refs/tags/0.1.3": B})
        self.assertEqual(self.run_sync(repository), 0)
        self.assertEqual(repository.fetches, [])
        self.assertEqual(repository.transfers, [])
        self.assertEqual(repository.merges, [])

    def test_new_peer_receives_main_first_and_gitea_release_tags(self):
        repository = FakeRepository({"refs/heads/z-feature": A, MAIN: B, "refs/tags/0.1.3": C}, {})
        self.assertEqual(self.run_sync(repository), 0)
        self.assertEqual([item[2] for item in repository.transfers], [MAIN, "refs/heads/z-feature", "refs/tags/0.1.3"])
        self.assertTrue(all(item[0:2] == ("gitea", "github") for item in repository.transfers))

    def test_new_branches_are_imported_but_peer_tags_are_not(self):
        repository = FakeRepository({MAIN: A}, {MAIN: A, "refs/heads/from-github": B, "refs/tags/8.8.8": C})
        self.assertEqual(self.run_sync(repository), 0)
        self.assertEqual(repository.transfers, [("github", "gitea", "refs/heads/from-github", B, None, False)])

    def test_fast_forward_runs_in_either_direction(self):
        for primary, peer, source, destination in [(B, A, "gitea", "github"), (A, B, "github", "gitea")]:
            with self.subTest(source=source):
                repository = FakeRepository({MAIN: primary}, {MAIN: peer}, ancestry=[(A, B)])
                self.assertEqual(self.run_sync(repository), 0)
                self.assertEqual(repository.transfers, [(source, destination, MAIN, B, A, False)])
                self.assertFalse(repository.merges)

    def test_divergent_histories_are_merged_or_left_for_review(self):
        for merge_result in [True, False]:
            with self.subTest(merge_result=merge_result):
                repository = FakeRepository({MAIN: A}, {MAIN: B}, merge_result=merge_result)
                self.assertEqual(self.run_sync(repository), 0 if merge_result else 1)
                self.assertEqual(repository.merges, [(MAIN, A, B, False)])
                self.assertEqual(repository.transfers, [])

    def test_existing_tag_conflict_does_not_overwrite_either_side(self):
        repository = FakeRepository({MAIN: A, "refs/tags/0.1.3": B}, {MAIN: A, "refs/tags/0.1.3": C})
        self.assertEqual(self.run_sync(repository), 1)
        self.assertEqual(repository.transfers, [])
        self.assertEqual(repository.fetches, [("gitea", {}), ("github", {})])

    def test_rolling_tags_are_never_synchronized(self):
        aliases = ["beta", "nightly", "beta-stage-" + A, "nightly-stage-" + B]
        repository = FakeRepository({MAIN: A, **{"refs/tags/" + tag: B for tag in aliases}}, {MAIN: A})
        self.assertEqual(self.run_sync(repository), 0)
        self.assertEqual(repository.fetches, [])
        self.assertEqual(repository.transfers, [])
        self.assertTrue(sync.included_tag("refs/tags/0.1.3"))
        self.assertFalse(sync.included_tag(MAIN))

    def test_absent_branch_is_copied_instead_of_deleted(self):
        repository = FakeRepository({MAIN: A, "refs/heads/only-primary": B}, {MAIN: A, "refs/heads/only-peer": C})
        self.assertEqual(self.run_sync(repository), 0)
        self.assertEqual({item[2] for item in repository.transfers}, {"refs/heads/only-primary", "refs/heads/only-peer"})
        self.assertTrue(all(item[3] in [B, C] for item in repository.transfers))

    def test_dry_run_is_propagated_to_all_proposed_changes(self):
        repository = FakeRepository({MAIN: B, "refs/tags/0.1.3": C}, {MAIN: A}, ancestry=[(A, B)])
        self.assertEqual(self.run_sync(repository, True), 0)
        self.assertTrue(all(item[-1] for item in repository.transfers))

    def test_lfs_precedes_nonforce_ref_publication_and_ref_is_rechecked(self):
        repository = TransferHarness([A, A, B])
        with contextlib.redirect_stdout(io.StringIO()):
            repository.push("gitea", "github", MAIN, B, A, False)
        self.assertEqual(repository.operations[:2], [("fetch-lfs", "gitea", B), ("push-lfs", "github", B)])
        command = repository.operations[2][1]
        self.assertEqual(command, ("push", "--porcelain", "--no-follow-tags", "peer", B + ":" + MAIN))
        self.assertFalse(any("force" in part for part in command))

    def test_races_before_or_after_lfs_do_not_publish_a_ref(self):
        for states, operation_count in [([C], 0), ([A, C], 2)]:
            repository = TransferHarness(states)
            with self.subTest(states=states), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(sync.SyncError, "changed"):
                    repository.push("gitea", "github", MAIN, B, A, False)
            self.assertEqual(len(repository.operations), operation_count)
            self.assertFalse(any(op[0] == "git" for op in repository.operations))

    def test_peer_import_needs_explicit_gitea_authority(self):
        repository = TransferHarness([A], token="")
        with self.assertRaisesRegex(sync.SyncError, "GITEA_TOKEN"):
            repository.push("github", "gitea", MAIN, B, A, False)
        self.assertEqual(repository.operations, [])

    def test_already_copied_ref_is_idempotent_without_transfers(self):
        repository = TransferHarness([B])
        with contextlib.redirect_stdout(io.StringIO()):
            repository.push("gitea", "github", MAIN, B, A, False)
        self.assertEqual(repository.operations, [])

    def test_dry_run_transfer_cannot_download_or_upload_lfs(self):
        repository = TransferHarness([A])
        with contextlib.redirect_stdout(io.StringIO()):
            repository.push("gitea", "github", MAIN, B, A, True)
        self.assertEqual(repository.operations, [])

    def test_only_the_configured_solar_repositories_are_reachable(self):
        self.assertEqual(sync.GITEA_URL, "https://git.ryserve.net/internal/solar-chat.git")
        self.assertEqual(sync.PEERS, {"github": "git@github.com:rkw6086/solar.git"})
        self.assertEqual(set(sync.LFS_ENDPOINTS), {"gitea", "github"})


if __name__ == "__main__":
    unittest.main()
