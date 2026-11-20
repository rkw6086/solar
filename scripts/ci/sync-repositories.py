#!/usr/bin/env python3
"""Reconcile Solar between Gitea and one configured repository peer.

Gitea owns release tags. Mutable beta and nightly aliases stay local to
their service and are never mirrored between repositories.
Clean divergent histories receive a merge commit, with Gitea as its first parent,
published to Gitea before the selected peer. Conflicts and unrelated histories are preserved
for maintainer review. This is deliberately not `git push --mirror`: deletions
are not propagated, and ordinary non-force pushes protect concurrent work.
Every ref publication first transfers all reachable Git LFS payloads; matching
refs remain a lightweight no-op without Git-object or LFS downloads.
No checked-out source code is executed by this job.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

GITEA_URL = "https://git.ryserve.net/internal/solar-chat.git"
PEERS = {
    "github": "git@github.com:rkw6086/solar.git",
}
PEER_LABELS = {"github": "GitHub"}
LFS_ENDPOINTS = {
    "gitea": "https://git.ryserve.net/internal/solar-chat.git/info/lfs",
    # Keep GitHub's SSH authentication exchange, which returns its HTTPS LFS
    # endpoint and short-lived authorization without storing a GitHub token.
    "github": "git@github.com:rkw6086/solar.git",
}
OBJECT_ID = re.compile(r"[0-9a-f]{40}\Z")
# This legacy blob contained a private credential and was removed from the
# public history. Reject an older clone that brings it back without reading or
# logging any blob contents. This is an exact object deny-list, not a scanner.
FORBIDDEN_PUBLIC_BLOBS = frozenset({"b6fffad8b893dd7b8b66d28ce3ea2bddf9d8450c"})
MAX_REFS = 10_000
MAX_OUTPUT = 4 * 1024 * 1024


class SyncError(Exception):
    pass


def report(result, **details):
    # JSON also escapes unusual ref names instead of emitting terminal controls.
    print(json.dumps({"result": result, **details}, sort_keys=True), flush=True)


class Repository:
    def __init__(self, directory, peer="github"):
        if peer not in PEERS:
            raise SyncError("Unknown repository synchronization peer.")
        self.peer = peer
        # Each invocation reconciles one pair only. The serialized Gitea jobs
        # propagate accepted changes between peers through the primary repo.
        self.remotes = {"gitea": GITEA_URL, peer: PEERS[peer]}
        self.directory = directory
        self.lfs_fetched = set()
        self.lfs_pushed = set()
        self.public_history_checked = set()
        self.env = {
            key: value for key, value in os.environ.items()
            if not key.startswith("GIT_") or key == "GIT_SSH_COMMAND"
        }
        self.env.update({
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_ASKPASS": str(directory / "askpass.py"),
            "GIT_AUTHOR_NAME": "Solar Repository Sync",
            "GIT_AUTHOR_EMAIL": "solar-sync@users.noreply.git.ryserve.net",
            "GIT_COMMITTER_NAME": "Solar Repository Sync",
            "GIT_COMMITTER_EMAIL": "solar-sync@users.noreply.git.ryserve.net",
            "LC_ALL": "C",
        })
        askpass = directory / "askpass.py"
        askpass.write_text(
            "#!/usr/bin/env python3\n"
            "import os, sys\n"
            "prompt = sys.argv[1].lower() if len(sys.argv) > 1 else ''\n"
            "if 'git.ryserve.net' not in prompt:\n"
            "    sys.exit(1)\n"
            "if 'username' in prompt:\n"
            "    print('x-access-token')\n"
            "elif 'password' in prompt:\n"
            "    print(os.environ.get('GITEA_TOKEN', ''))\n"
            "else:\n"
            "    sys.exit(1)\n"
        )
        askpass.chmod(0o700)
        self.git("init", "--bare", "repository", operation="initialize")
        self.directory = directory / "repository"
        # Named remotes let LFS resolve the same trusted identities as Git.
        # Neither URLs nor configuration contain credentials.
        for remote, url in self.remotes.items():
            self.git("remote", "add", remote, url, operation=f"configure {remote}")

    def git(self, *args, operation, allowed=(0,), timeout=300):
        # Never expose transport output: credential helpers or remote hook errors
        # can contain credentials. There is no shell interpolation or checkout.
        command = [
            "git", "-c", "core.hooksPath=" + os.devnull,
            "-c", "credential.helper=", "-c", "protocol.file.allow=never",
            "-c", "protocol.ext.allow=never", "-c", "http.followRedirects=false",
            *args,
        ]
        with tempfile.TemporaryFile() as output, tempfile.TemporaryFile() as errors:
            try:
                result = subprocess.run(
                    command, cwd=self.directory, env=self.env,
                    stdin=subprocess.DEVNULL, stdout=output, stderr=errors,
                    timeout=timeout, check=False,
                )
            except subprocess.TimeoutExpired as error:
                raise SyncError(f"{operation}: timed out; retry the next scheduled sync.") from error
            if result.returncode not in allowed:
                errors.seek(0)
                diagnostic = errors.read(MAX_OUTPUT).decode("utf-8", errors="replace").lower()
                # Classify known failures without exposing provider output,
                # credential-helper text, signed URLs, or repository content.
                if "workflow" in diagnostic and "scope" in diagnostic and "refusing to allow" in diagnostic:
                    raise SyncError(f"{operation}: GitHub requires workflow permission for the credential that registered the deploy key.")
                if "gh008" in diagnostic:
                    raise SyncError(f"{operation}: GitHub rejected missing Git LFS payloads.")
                if "host key verification failed" in diagnostic:
                    raise SyncError(f"{operation}: SSH host key verification failed.")
                raise SyncError(f"{operation}: Git failed with exit status {result.returncode}.")
            output.seek(0)
            data = output.read(MAX_OUTPUT + 1)
            if len(data) > MAX_OUTPUT:
                raise SyncError(f"{operation}: Git output exceeded the safe limit.")
        try:
            return result.returncode, data.decode("utf-8")
        except UnicodeDecodeError as error:
            raise SyncError(f"{operation}: Git returned non-UTF-8 ref data.") from error

    def refs(self, remote, ref=None):
        patterns = [ref] if ref is not None else ["refs/heads/*", "refs/tags/*"]
        _, output = self.git(
            "ls-remote", "--refs", self.remotes[remote], *patterns,
            operation=f"read {remote} refs", timeout=60,
        )
        result = {}
        for line in output.splitlines():
            fields = line.split("\t")
            if len(fields) != 2 or not OBJECT_ID.fullmatch(fields[0]):
                raise SyncError(f"{remote}: malformed ref advertisement.")
            oid, name = fields
            if not name.startswith(("refs/heads/", "refs/tags/")):
                raise SyncError(f"{remote}: unexpected advertised ref namespace.")
            self.git("check-ref-format", name, operation=f"validate {remote} ref")
            if name in result:
                raise SyncError(f"{remote}: duplicate advertised ref.")
            result[name] = oid
            if len(result) > MAX_REFS:
                raise SyncError(f"{remote}: too many refs for one synchronization.")
        # ls-remote patterns are globs, so callers still select an exact ref.
        return result

    def fetch(self, remote, refs):
        if not refs:
            return
        self.git(
            "fetch", "--no-tags", "--no-recurse-submodules",
            "--no-write-fetch-head", self.remotes[remote],
            *(f"+{ref}:refs/solar-sync/{remote}/{ref[5:]}" for ref in sorted(refs)),
            operation=f"fetch {remote}", timeout=600,
        )
        for ref, expected in refs.items():
            _, actual = self.git(
                "rev-parse", "--verify", f"refs/solar-sync/{remote}/{ref[5:]}",
                operation=f"verify fetched {remote} ref",
            )
            if actual.strip() != expected:
                raise SyncError(f"{remote}: refs changed while fetching; defer to the next sync.")

    def ancestor(self, older, newer):
        status, _ = self.git(
            "merge-base", "--is-ancestor", older, newer,
            operation="compare branch ancestry", allowed=(0, 1),
        )
        return status == 0

    def lfs(self, operation, remote, oid):
        if remote not in self.remotes or not OBJECT_ID.fullmatch(oid):
            raise SyncError("Invalid Git LFS transfer identity.")
        endpoint = LFS_ENDPOINTS[remote]
        self.git(
            "-c", f"lfs.url={endpoint}", "-c", f"lfs.pushurl={endpoint}",
            "-c", f"remote.{remote}.lfsurl={endpoint}",
            "-c", f"remote.{remote}.lfspushurl={endpoint}",
            "-c", "lfs.remote.autodetect=false", "-c", "lfs.remote.searchall=false",
            "-c", "lfs.basictransfersonly=true", "-c", "lfs.allowincompletepush=false",
            "-c", "lfs.concurrenttransfers=4", "-c", "lfs.transfer.maxretries=2",
            "lfs", operation, "--all", remote, oid,
            operation=f"{operation} Git LFS payloads on {remote}", timeout=600,
        )

    def fetch_lfs(self, remote, oid):
        identity = (remote, oid)
        if identity not in self.lfs_fetched:
            self.lfs("fetch", remote, oid)
            self.lfs_fetched.add(identity)
            report("lfs-fetched", source=remote, oid=oid)

    def push_lfs(self, remote, oid):
        identity = (remote, oid)
        if identity not in self.lfs_pushed:
            self.lfs("push", remote, oid)
            self.lfs_pushed.add(identity)
            report("lfs-pushed", destination=remote, oid=oid)

    def check_public_history(self, oid):
        if not OBJECT_ID.fullmatch(oid):
            raise SyncError("Invalid public source identity.")
        if oid in self.public_history_checked:
            return
        # Check the entire reachable history, not just today's tree: a revert
        # or merge from a pre-scrub clone must not republish the removed blob.
        # The existing Git output bound and this timeout fail closed if the
        # graph is too large or cannot be read. Object paths/content stay out
        # of both the command output and workflow logs.
        _, output = self.git(
            "rev-list", "--objects", "--no-object-names", oid, "--",
            operation="check public history", timeout=60,
        )
        if not output.strip():
            raise SyncError("Public history did not return any object identities.")
        for item in output.splitlines():
            if not OBJECT_ID.fullmatch(item):
                raise SyncError("Public history returned an invalid object identity.")
            if item in FORBIDDEN_PUBLIC_BLOBS:
                raise SyncError("GitHub publication blocked: source history contains a protected private credential object. Remove that history before retrying.")
        self.public_history_checked.add(oid)

    def merge(self, ref, gitea, peer_oid, dry_run):
        status, _ = self.git(
            "merge-base", gitea, peer_oid,
            operation="find branch merge base", allowed=(0, 1),
        )
        if status == 1:
            report("conflict", ref=ref, reason="unrelated-history", **{"gitea": gitea, self.peer: peer_oid})
            return False
        # merge-tree uses the built-in merge engine in the bare object database.
        # No remote worktree, hooks, custom merge driver or source code is run.
        status, output = self.git(
            "merge-tree", "--write-tree", "--no-messages", gitea, peer_oid,
            operation="merge branch histories", allowed=(0, 1),
        )
        if status == 1:
            report("conflict", ref=ref, reason="merge-conflict", **{"gitea": gitea, self.peer: peer_oid})
            return False
        tree = output.strip()
        if not OBJECT_ID.fullmatch(tree):
            raise SyncError("Merge engine returned an unexpected tree identity.")
        if dry_run:
            report("would-merge", peer=self.peer, ref=ref, first_parent=gitea, second_parent=peer_oid, tree=tree)
            return True
        _, output = self.git(
            "commit-tree", tree, "-p", gitea, "-p", peer_oid,
            "-m", f"Merge {PEER_LABELS[self.peer]} {ref[11:]} into Gitea",
            operation="record clean merge",
        )
        commit = output.strip()
        if not OBJECT_ID.fullmatch(commit):
            raise SyncError("Merge commit returned an unexpected identity.")
        # A successful first publication is durable even if the peer races or is
        # unavailable; the next job sees the commit and resumes reconciliation.
        sources = (("gitea", gitea), (self.peer, peer_oid))
        self.push("clean-merge", "gitea", ref, commit, gitea, False, lfs_sources=sources)
        self.push("clean-merge", self.peer, ref, commit, peer_oid, False, lfs_sources=sources)
        report("merged", peer=self.peer, ref=ref, oid=commit)
        return True

    def push(self, source, destination, ref, oid, expected, dry_run, lfs_sources=None):
        current = self.refs(destination, ref).get(ref)
        if current == oid:
            report("already-synchronized", ref=ref, destination=destination)
            return
        if current != expected:
            raise SyncError(f"{destination}: destination changed before push; defer to the next sync.")
        if destination == "gitea" and not self.env.get("GITEA_TOKEN"):
            if not dry_run:
                raise SyncError(f"GITEA_TOKEN is required to import {PEER_LABELS[self.peer]} changes.")
        if destination == "github":
            # Run before LFS or Git writes, including during a dry-run preview.
            self.check_public_history(oid)
        if dry_run:
            report("would-copy", source=source, destination=destination, ref=ref, oid=oid)
            return
        # LFS hooks are disabled intentionally. Explicit transfers cover the
        # entire reachable history, including both parents of a clean merge,
        # before making a ref visible to clients of the destination repository.
        for owner, source_oid in lfs_sources or ((source, oid),):
            self.fetch_lfs(owner, source_oid)
        self.push_lfs(destination, oid)
        # A transfer can take time; repeat the freshness check before Git push.
        current = self.refs(destination, ref).get(ref)
        if current == oid:
            report("already-synchronized", ref=ref, destination=destination)
            return
        if current != expected:
            raise SyncError(f"{destination}: destination changed during LFS transfer; defer to the next sync.")
        # An ordinary non-force push is the final server-side ancestry guard.
        # A racing divergent update is rejected. A racing fast-forward that is
        # still an ancestor of oid is harmless and may be advanced normally.
        self.git(
            "push", "--porcelain", "--no-follow-tags", self.remotes[destination],
            f"{oid}:{ref}", operation=f"push to {destination}", timeout=600,
        )
        actual = self.refs(destination, ref).get(ref)
        if actual != oid:
            raise SyncError(f"{destination}: ref changed after push; verify on the next sync.")
        report("copied", source=source, destination=destination, ref=ref, oid=oid)


def included_tag(ref):
    return (ref.startswith("refs/tags/")
            and ref not in ("refs/tags/beta", "refs/tags/nightly")
            and not ref.startswith(("refs/tags/beta-stage-", "refs/tags/nightly-stage-")))


def synchronize(repository, dry_run):
    peer = repository.peer
    snapshots = {remote: repository.refs(remote) for remote in repository.remotes}
    branches = sorted(
        {ref for refs in snapshots.values() for ref in refs if ref.startswith("refs/heads/")},
        # On an empty peer the first pushed branch can become its default.
        key=lambda ref: (ref != "refs/heads/main", ref),
    )
    tags = sorted(ref for ref in snapshots["gitea"] if included_tag(ref))
    pending = {remote: {} for remote in repository.remotes}
    differing_branches = []
    differing_tags = []
    for ref in branches:
        left, right = (snapshots[remote].get(ref) for remote in repository.remotes)
        if left != right:
            differing_branches.append(ref)
            for remote in repository.remotes:
                if ref in snapshots[remote]:
                    pending[remote][ref] = snapshots[remote][ref]
    for ref in tags:
        if snapshots["gitea"][ref] != snapshots[peer].get(ref):
            differing_tags.append(ref)
            if ref not in snapshots[peer]:
                pending["gitea"][ref] = snapshots["gitea"][ref]
    if not differing_branches and not differing_tags:
        report("synchronized", peer=peer, branches=len(branches), release_tags=len(tags), copied=0)
        return 0
    for remote, refs in pending.items():
        repository.fetch(remote, refs)
    failures = 0
    for ref in differing_branches:
        left, right = (snapshots[remote].get(ref) for remote in repository.remotes)
        try:
            if right is None or (left is not None and repository.ancestor(right, left)):
                source, destination, oid, expected = "gitea", peer, left, right
            elif left is None or repository.ancestor(left, right):
                source, destination, oid, expected = peer, "gitea", right, left
            else:
                if not repository.merge(ref, left, right, dry_run):
                    failures += 1
                continue
            repository.push(source, destination, ref, oid, expected, dry_run)
        except SyncError as error:
            report("failed", ref=ref, reason=str(error))
            failures += 1
    for ref in differing_tags:
        try:
            if ref in snapshots[peer]:
                report("conflict", ref=ref, reason="immutable-tag-mismatch",
                       **{"gitea": snapshots["gitea"][ref], peer: snapshots[peer][ref]})
                failures += 1
                continue
            repository.push("gitea", peer, ref, snapshots["gitea"][ref], None, dry_run)
        except SyncError as error:
            report("failed", ref=ref, reason=str(error))
            failures += 1
    report("finished", peer=peer, differing_branches=len(differing_branches), differing_tags=len(differing_tags), failures=failures, dry_run=dry_run)
    return 1 if failures else 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer", choices=sorted(PEERS), default="github",
                        help="Reconcile this configured peer with the Gitea primary (default: github).")
    parser.add_argument("--dry-run", action="store_true", help="Show safe updates without pushing.")
    args = parser.parse_args()
    try:
        with tempfile.TemporaryDirectory(prefix=f"solar-{args.peer}-sync-") as directory:
            return synchronize(Repository(Path(directory), args.peer), args.dry_run)
    except (OSError, SyncError) as error:
        # Do not include arbitrary OS/subprocess error text or environment values.
        reason = str(error) if isinstance(error, SyncError) else "Local Git synchronization setup failed."
        report("failed", reason=reason)
        return 1


if __name__ == "__main__":
    sys.exit(main())
