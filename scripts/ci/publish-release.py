#!/usr/bin/env python3
"""Publish verified release files with the short-lived Gitea Actions job token."""
from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
from dataclasses import dataclass, field
import urllib.error
import urllib.parse
import urllib.request

VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z")
MAX_ASSET_BYTES = 512 * 1024 * 1024
COMMIT = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")
STAGE = re.compile(r"beta-stage-([0-9a-f]{40}|[0-9a-f]{64})-[0-9a-f]{16}\Z")
ROOT = Path(__file__).resolve().parents[2]


class PublishError(Exception):
    pass


class AssetMismatch(PublishError):
    pass


class MissingResource(PublishError):
    pass


@dataclass(frozen=True)
class Config:
    version: str
    tag: str
    directory: Path
    server: str
    repository: str
    commit: str
    token: str = field(repr=False)
    notes: str | None = None
    channel: str = "release"
    previous_beta: str | None = None

    @classmethod
    def from_env(cls, env):
        version = env.get("SOLAR_RELEASE_VERSION", "")
        channel = env.get("SOLAR_BUILD_CHANNEL", "release")
        tags = {env[name] for name in ("RELEASE_TAG", "SOLAR_RELEASE_TAG") if env.get(name)}
        if channel not in ("release", "beta") or not VERSION.fullmatch(version):
            raise PublishError("Use a stable package version and beta or release channel.")
        if channel == "release" and tags != {version}:
            raise PublishError("Use an exact bare stable version tag matching SOLAR_RELEASE_VERSION.")
        if channel == "beta" and tags not in (set(), {"beta"}):
            raise PublishError("Beta publication cannot use a numeric release tag.")
        commit = env.get("SOLAR_RELEASE_SHA", env.get("SOLAR_SOURCE_COMMIT", ""))
        if not COMMIT.fullmatch(commit) or set(commit) == {"0"}:
            raise PublishError("SOLAR_RELEASE_SHA must identify the release commit.")
        repository = env.get("GITEA_REPOSITORY", "")
        if (not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)
                or any(segment in (".", "..") for segment in repository.split("/"))):
            raise PublishError("GITEA_REPOSITORY must be owner/repository.")
        server = env.get("GITEA_SERVER_URL", "").rstrip("/")
        parsed = urllib.parse.urlsplit(server)
        if (parsed.scheme != "https" or not parsed.hostname or parsed.username
                or parsed.password or parsed.query or parsed.fragment or parsed.path):
            raise PublishError("GITEA_SERVER_URL must be an HTTPS server URL without credentials.")
        token = env.get("GITEA_TOKEN", "")
        if not token or any(character.isspace() for character in token):
            raise PublishError("GITEA_TOKEN is missing or malformed.")
        directory = env.get("SOLAR_RELEASE_DIR", "")
        if not directory or not Path(directory).is_dir():
            raise PublishError("SOLAR_RELEASE_DIR must contain the verified release files.")
        notes = None
        if env.get("SOLAR_RELEASE_NOTES_FILE"):
            path = Path(env["SOLAR_RELEASE_NOTES_FILE"])
            if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 128 * 1024:
                raise PublishError("Release notes must be a bounded regular file.")
            notes = path.read_text(encoding="utf-8")
        previous = env.get("SOLAR_PREVIOUS_BETA_SHA") or None
        if previous and not COMMIT.fullmatch(previous):
            raise PublishError("Previous beta identity must be an exact commit.")
        return cls(version, "beta" if channel == "beta" else version, Path(directory),
                   server, repository, commit, token, notes, channel, previous)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Gitea:
    def __init__(self, config, opener=None):
        self.config = config
        self.opener = opener or urllib.request.build_opener(NoRedirect())
        self.origin = urllib.parse.urlsplit(config.server)[:2]
        self.api = config.server + "/api/v1/repos/" + config.repository
        self.verified_asset_digests = {}

    def request(self, method, path, body=None, upload=None, missing_ok=False):
        headers = {"Authorization": "token " + self.config.token, "Accept": "application/json"}
        data = None
        if upload is not None:
            boundary = "solar-" + secrets.token_hex(24)
            # Filenames come from validated versions or exact beta source commits.
            prefix = (f'--{boundary}\r\nContent-Disposition: form-data; name="attachment"; '
                      f'filename="{upload.name}"\r\nContent-Type: application/octet-stream\r\n\r\n').encode()
            data = prefix + upload.read_bytes() + f"\r\n--{boundary}--\r\n".encode()
            headers["Content-Type"] = "multipart/form-data; boundary=" + boundary
        elif body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.api + path, data=data, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=300 if upload is not None else 60) as response:
                data = response.read(4 * 1024 * 1024 + 1)
                if len(data) > 4 * 1024 * 1024:
                    raise PublishError("Release API returned an oversized response.")
                return json.loads(data) if data else None
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            if status == 404 and missing_ok:
                return None
            if status == 404:
                raise MissingResource("Gitea release API resource was not found.") from None
            raise PublishError(f"Gitea release API returned HTTP {status}; reconcile retained release state before retrying.") from None
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
            raise PublishError("Gitea release API request failed; a write may have completed. Retry only through state reconciliation.") from None

    def download_bytes(self, url, limit):
        for _ in range(6):
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
                raise PublishError("Release asset download is not a safe HTTPS URL.")
            headers = {"Authorization": "token " + self.config.token} if parsed[:2] == self.origin else {}
            try:
                with self.opener.open(urllib.request.Request(url, headers=headers), timeout=60) as response:
                    data = response.read(limit + 1)
                    if len(data) > limit:
                        raise PublishError("Release checksum download exceeded its bound.")
                    return data
            except urllib.error.HTTPError as error:
                status, location = error.code, error.headers.get("Location")
                error.close()
                if status in (301, 302, 303, 307, 308) and location:
                    url = urllib.parse.urljoin(url, location)
                    continue
                raise PublishError(f"Release checksum download returned HTTP {status}.") from None
            except (urllib.error.URLError, TimeoutError):
                raise PublishError("Release checksum download failed; URL and credentials withheld.") from None
        raise PublishError("Release checksum download exceeded the redirect limit.")

    def download_digest(self, url, expected_size):
        # Downloads may redirect to signed object-storage URLs. Never forward the job
        # token to another origin, or expose the signed redirect URL in errors/logs.
        for _ in range(6):
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
                raise PublishError("Release asset download is not a safe HTTPS URL.")
            headers = {}
            if parsed[:2] == self.origin:
                headers["Authorization"] = "token " + self.config.token
            request = urllib.request.Request(url, headers=headers)
            try:
                with self.opener.open(request, timeout=60) as response:
                    digest = hashlib.sha256()
                    size = 0
                    while chunk := response.read(1024 * 1024):
                        size += len(chunk)
                        if size > expected_size:
                            raise PublishError("Release asset size exceeds the local file.")
                        digest.update(chunk)
                    if size != expected_size:
                        raise PublishError("Release asset size does not match the local file.")
                    return digest.hexdigest()
            except urllib.error.HTTPError as error:
                status, location = error.code, error.headers.get("Location")
                error.close()
                if status in (301, 302, 303, 307, 308) and location:
                    url = urllib.parse.urljoin(url, location)
                    continue
                raise PublishError(f"Release asset download returned HTTP {status}.") from None
            except (urllib.error.URLError, TimeoutError):
                raise PublishError("Release asset download failed; URL and credentials withheld.") from None
        raise PublishError("Release asset download exceeded the redirect limit.")


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def release_files(config):
    identity = "beta_" + config.commit if config.channel == "beta" else config.version
    dmg = config.directory / f"Solar_{identity}_aarch64.dmg"
    checksum = config.directory / "SHA256SUMS"
    for path in (dmg, checksum):
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= MAX_ASSET_BYTES:
            raise PublishError("Expected nonempty regular DMG and checksum files within the upload limit.")
    if checksum.stat().st_size > 4096:
        raise PublishError("SHA256SUMS is unexpectedly large.")
    try:
        lines = checksum.read_text(encoding="ascii").splitlines()
    except UnicodeError:
        raise PublishError("SHA256SUMS is not valid ASCII.") from None
    expected = file_digest(dmg)
    if len(lines) != 1 or not re.fullmatch(re.escape(expected) + r" [ *]" + re.escape(dmg.name), lines[0]):
        raise PublishError("The DMG does not match SHA256SUMS.")
    return {path.name: (path, file_digest(path)) for path in (dmg, checksum)}


def notes_from_changelog(version, path):
    try:
        content = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        raise PublishError("CHANGELOG.md is missing or unreadable.") from None
    headings = list(re.finditer(r"(?m)^##[ \t]+\[" + re.escape(version) + r"\](?:[ \t]+[^\r\n]*)?[ \t]*$", content))
    if len(headings) != 1:
        raise PublishError("CHANGELOG.md must contain exactly one section for the release version.")
    remainder = content[headings[0].end():]
    end = re.search(r"(?m)^##[ \t]+", remainder)
    notes = remainder[:end.start() if end else len(remainder)].strip()
    if not notes:
        raise PublishError("The release's CHANGELOG.md section is empty.")
    return notes


def tag_path(tag):
    return "/tags/" + urllib.parse.quote(tag, safe="")


def release_path(tag):
    return "/releases/tags/" + urllib.parse.quote(tag, safe="")


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def list_all(client, path, key=None):
    result = []
    for page in range(1, 201):
        value = client.request("GET", path + ("&" if "?" in path else "?") + f"limit=50&page={page}")
        values = value.get(key) if key and isinstance(value, dict) else value
        if not isinstance(values, list):
            raise PublishError("Unexpected paginated release API response.")
        result.extend(values)
        if len(values) < 50:
            return result
    raise PublishError("Release API listing exceeded its safe bound.")


def owned_identity(release):
    tag, body = release.get("tag_name", ""), release.get("body", "")
    if VERSION.fullmatch(tag) and not release.get("prerelease"):
        pattern = r"<!-- solar-release:v1 tag=" + re.escape(tag) + r" commit=([0-9a-f]{40}|[0-9a-f]{64}) -->"
        matches = re.findall(pattern, body)
        channel = "release"
    elif (tag == "beta" or STAGE.fullmatch(tag)) and release.get("prerelease"):
        matches = re.findall(r"<!-- solar-beta:v1 commit=([0-9a-f]{40}|[0-9a-f]{64}) -->", body)
        channel = "beta"
    else:
        return None
    if len(matches) != 1 or release.get("target_commitish") != matches[0]:
        return None
    if STAGE.fullmatch(tag) and STAGE.fullmatch(tag)[1] != matches[0]:
        return None
    return channel, matches[0]


def require_owned(release, channel, commit, tag):
    if release.get("tag_name") != tag or owned_identity(release) != (channel, commit):
        raise PublishError("Existing release does not match this generated channel and source; left unchanged.")


def check_tag(client, tag, commit):
    value = client.request("GET", tag_path(tag))
    if value.get("name") != tag or value.get("commit", {}).get("sha") != commit:
        raise PublishError("Release tag no longer points to the expected commit.")


def release_assets(client, release):
    values = list_all(client, f"/releases/{release['id']}/assets")
    names = [asset.get("name") for asset in values]
    if len(set(names)) != len(names):
        raise PublishError("Release contains duplicate asset names; left unchanged.")
    return {asset["name"]: asset for asset in values}


def asset_digest(client, asset):
    # Gitea attachment IDs identify immutable file content. Renaming/promoting
    # a release changes its download URL but does not require another 250+ MiB
    # download of the same attachment. Re-read asset metadata at each boundary;
    # a replacement upload has a new ID and must be verified independently.
    cache = getattr(client, "verified_asset_digests", None)
    key = (asset.get("id"), asset.get("size"))
    cacheable = isinstance(key[0], int) and key[0] > 0 and isinstance(cache, dict)
    if cacheable and key in cache:
        return cache[key]
    value = client.download_digest(asset.get("browser_download_url", ""), asset["size"])
    if cacheable:
        cache[key] = value
    return value


def local_assets(client, release, files, complete=False):
    found = release_assets(client, release)
    if not set(found) <= set(files) or (complete and set(found) != set(files)):
        raise AssetMismatch("Release contains unexpected or missing assets; left unchanged.")
    for name, asset in found.items():
        path, digest = files[name]
        if asset.get("size") != path.stat().st_size:
            raise AssetMismatch("Existing release asset has different content; left unchanged.")
        remote = asset_digest(client, asset)
        if not hmac.compare_digest(remote, digest):
            raise AssetMismatch("Existing release asset checksum differs; left unchanged.")
    return found


def remote_healthy(client, release):
    identity = owned_identity(release)
    if identity is None or release.get("draft"):
        raise PublishError("Retention requires a published Solar-owned replacement.")
    channel, commit = identity
    tag = release["tag_name"]
    check_tag(client, tag, commit)
    name = f"Solar_beta_{commit}_aarch64.dmg" if channel == "beta" else f"Solar_{tag}_aarch64.dmg"
    assets = release_assets(client, release)
    if set(assets) != {name, "SHA256SUMS"}:
        raise PublishError("Published replacement does not have its exact DMG and checksum pair.")
    dmg, checksum = assets[name], assets["SHA256SUMS"]
    if not isinstance(dmg.get("size"), int) or not 0 < dmg["size"] <= MAX_ASSET_BYTES:
        raise PublishError("Replacement DMG size is invalid.")
    if not isinstance(checksum.get("size"), int) or not 0 < checksum["size"] <= 4096:
        raise PublishError("Replacement checksum size is invalid.")
    data = client.download_bytes(checksum.get("browser_download_url", ""), 4096)
    try:
        lines = data.decode("ascii").splitlines()
    except UnicodeError:
        raise PublishError("Replacement checksum is not ASCII.") from None
    match = re.fullmatch(r"([0-9a-f]{64}) [ *]" + re.escape(name), lines[0]) if len(lines) == 1 else None
    if not match or len(data) != checksum["size"]:
        raise PublishError("Replacement checksum does not name its exact DMG.")
    digest = asset_digest(client, dmg)
    if not hmac.compare_digest(digest, match[1]):
        raise PublishError("Replacement remote DMG failed checksum verification.")
    return assets


def release_body(config, notes):
    marker = (f"<!-- solar-beta:v1 commit={config.commit} -->" if config.channel == "beta"
              else f"<!-- solar-release:v1 tag={config.tag} commit={config.commit} -->")
    return (notes.strip() + "\n\nDownload the DMG below. Its SHA-256 checksum is supplied in SHA256SUMS.\n\n"
            + f"Source commit: `{config.commit}`\n\n{marker}")


def draft_with_files(config, client, tag, body, files):
    release = client.request("GET", release_path(tag), missing_ok=True)
    if release is None:
        release = client.request("POST", "/releases", body={
            "tag_name": tag, "target_commitish": config.commit, "draft": True,
            "prerelease": config.channel == "beta",
            "name": "Solar Beta" if config.channel == "beta" else "Solar " + config.version,
            "body": body,
        })
    require_owned(release, config.channel, config.commit, tag)
    found = local_assets(client, release, files)
    if not release.get("draft") and set(found) != set(files):
        raise PublishError("Published release is incomplete; it will not be overwritten.")
    for name, (path, _) in files.items():
        if name not in found:
            client.request("POST", f"/releases/{release['id']}/assets?name=" + urllib.parse.quote(name, safe=""), upload=path)
            found = local_assets(client, release, files)
    local_assets(client, release, files, complete=True)
    return client.request("GET", f"/releases/{release['id']}")


def delete_owned_release(client, release, before_delete=None):
    identity = owned_identity(release)
    if identity is None:
        raise PublishError("Unrecognized release cannot be removed by retention.")
    current = client.request("GET", f"/releases/{release['id']}", missing_ok=True)
    if current is None:
        return
    if (owned_identity(current) != identity or current.get("tag_name") != release["tag_name"]
            or current.get("draft") != release.get("draft")):
        raise PublishError("Release changed before retention; left unchanged.")
    channel, commit = identity
    allowed = {"SHA256SUMS", f"Solar_beta_{commit}_aarch64.dmg" if channel == "beta"
               else f"Solar_{release['tag_name']}_aarch64.dmg"}
    if not set(release_assets(client, current)) <= allowed:
        raise PublishError("Release has unrecognized attachments; retention stopped.")
    before_tag = client.request("GET", tag_path(release["tag_name"]), missing_ok=True)
    if before_tag and before_tag.get("commit", {}).get("sha") != commit:
        raise PublishError("Old release tag changed before retention; left unchanged.")
    # Revalidate retained downloads after inspecting the exact deletion target.
    if before_delete is not None:
        before_delete()
    # Gitea's release endpoint removes attachments, but retains the Git tag.
    client.request("DELETE", f"/releases/{release['id']}")
    if client.request("GET", f"/releases/{release['id']}", missing_ok=True) is not None:
        raise PublishError("Release deletion was not confirmed.")
    if before_tag:
        check_tag(client, release["tag_name"], commit)


def delete_beta_tag(client, tag, commit, tolerate_unindexed=False):
    if tag != "beta" and not STAGE.fullmatch(tag):
        raise PublishError("Only rolling beta and its staging tags may be removed.")
    if tolerate_unindexed and (tag != "beta" or commit != client.config.commit):
        raise PublishError("Unindexed alias recovery is restricted to this exact new beta source.")
    value = client.request("GET", tag_path(tag), missing_ok=True)
    if value is None:
        return
    check_tag(client, tag, commit)
    if client.request("GET", release_path(tag), missing_ok=True) is not None:
        raise PublishError("Cannot remove a beta tag still attached to a release.")
    unindexed = False
    try:
        client.request("DELETE", tag_path(tag))
    except MissingResource:
        if tag == "beta" and not tolerate_unindexed:
            raise
        unindexed = True
    remaining = client.request("GET", tag_path(tag), missing_ok=True)
    if remaining is not None:
        if remaining.get("commit", {}).get("sha") != commit:
            raise PublishError("Beta tag changed during deletion; cutover stopped.")
        if tag != "beta":
            # Renaming a release can leave an unindexed staging Git ref. It has
            # no release or assets, and Gitea's tag endpoint may return 404 for
            # it. Retain that tiny source ref rather than manufacture a release.
            print("Retained internal beta staging source ref without assets: " + tag)
            return
        if unindexed and tolerate_unindexed and client.request("GET", release_path("beta"), missing_ok=True) is None:
            # An interrupted promotion can create the exact new Git alias
            # before changing the staging release's DB record. Delete an
            # indexed IsTag alias first to avoid a unique-tag DB collision;
            # a 404 with the exact ref still present proves it is unindexed
            # and may instead be reused by the resumed release update.
            print("Reusing the exact unindexed beta source ref during publication recovery.")
            return
        raise PublishError("Beta tag deletion was not confirmed.")


def cleanup_stage_tag(client, tag, commit):
    try:
        delete_beta_tag(client, tag, commit)
    except PublishError:
        print("Internal beta staging ref cleanup deferred; verified public downloads remain available.")


def legacy_artifact_run(artifact, run):
    """Recognize the original manually dispatched, unversioned Solar archive."""
    source = artifact.get("workflow_run", {}).get("head_sha", "")
    return (artifact.get("name") == "Solar-macos-arm64"
            and isinstance(source, str) and COMMIT.fullmatch(source)
            and set(source) != {"0"} and run.get("head_sha") == source
            and run.get("event") == "workflow_dispatch"
            and run.get("path", "").removeprefix(".gitea/workflows/") == "macos-release.yml@refs/heads/main"
            and run.get("status") == "completed" and run.get("conclusion") == "success")


def prune_artifacts(client, before_delete=None):
    stable_name = re.compile(r"Solar-((?:(?:0|[1-9][0-9]*)\.){2}(?:0|[1-9][0-9]*))-macos-arm64(-recovery)?\Z")
    beta_name = re.compile(r"Solar-beta-([0-9a-f]{40}|[0-9a-f]{64})-([0-9]+)-macos-arm64\Z")
    for artifact in list_all(client, "/actions/artifacts", "artifacts"):
        stable = stable_name.fullmatch(artifact.get("name", ""))
        beta = beta_name.fullmatch(artifact.get("name", ""))
        legacy = artifact.get("name") == "Solar-macos-arm64"
        if not stable and not beta and not legacy:
            continue
        if artifact.get("expired"):
            print("Expired Solar Actions artifact is unavailable; Gitea background cleanup owns its stored bytes: " + artifact["name"])
            continue
        run_id = artifact.get("workflow_run", {}).get("id")
        if not isinstance(run_id, int) or run_id <= 0:
            continue
        run = client.request("GET", f"/actions/runs/{run_id}")
        path = run.get("path", "").removeprefix(".gitea/workflows/")
        # Match both the archive name and the exact workflow identity. A
        # similarly named attachment from another job is never ours to remove.
        known_workflow = (
            stable is not None and path == f"macos-release.yml@refs/tags/{stable[1]}"
            or stable is not None and stable[2] is not None
            and path == "macos-release-recovery.yml@refs/heads/main"
            or beta is not None and path == "macos-beta.yml@refs/heads/main"
            and run.get("head_sha") == beta[1] and run_id == int(beta[2])
            or legacy and legacy_artifact_run(artifact, run)
        )
        if run.get("status") != "completed" or not known_workflow:
            continue
        current = client.request("GET", f"/actions/artifacts/{artifact['id']}", missing_ok=True)
        if current is None:
            continue
        if current.get("expired"):
            print("Expired Solar Actions artifact is unavailable; Gitea background cleanup owns its stored bytes: " + artifact["name"])
            continue
        if current.get("name") != artifact["name"] or current.get("workflow_run", {}).get("id") != run_id:
            raise PublishError("Actions artifact changed before retention; left unchanged.")
        latest_run = client.request("GET", f"/actions/runs/{run_id}")
        if latest_run.get("status") != "completed":
            continue
        if legacy and not legacy_artifact_run(current, latest_run):
            raise PublishError("Legacy Actions archive identity changed before retention; left unchanged.")
        if before_delete is not None:
            before_delete()
        client.request("DELETE", f"/actions/artifacts/{artifact['id']}")
        remaining = client.request("GET", f"/actions/artifacts/{artifact['id']}", missing_ok=True)
        if remaining is not None and not remaining.get("expired"):
            raise PublishError("Actions artifact deletion was not confirmed.")


def retain_latest(client):
    releases = list_all(client, "/releases")
    stable = [release for release in releases if owned_identity(release)
              and owned_identity(release)[0] == "release" and not release.get("draft")]
    if not stable:
        return
    latest = max(stable, key=lambda release: version_key(release["tag_name"]))
    remote_healthy(client, latest)
    beta = client.request("GET", release_path("beta"), missing_ok=True)
    if beta is not None:
        remote_healthy(client, beta)
    def check_replacements():
        # A manual edit/deletion outside the serialized native workflow must
        # stop pruning rather than leave the user without verified downloads.
        current = client.request("GET", f"/releases/{latest['id']}", missing_ok=True)
        if current is None or current.get("tag_name") != latest["tag_name"] or owned_identity(current) != owned_identity(latest):
            raise PublishError("Retained stable release changed before cleanup; retention stopped.")
        remote_healthy(client, current)
        if beta is not None:
            current_beta = client.request("GET", release_path("beta"), missing_ok=True)
            if current_beta is None or current_beta.get("id") != beta["id"] or owned_identity(current_beta) != owned_identity(beta):
                raise PublishError("Retained beta release changed before cleanup; retention stopped.")
            remote_healthy(client, current_beta)

    for release in releases:
        identity = owned_identity(release)
        if identity is None:
            continue
        if identity[0] == "release" and version_key(release["tag_name"]) < version_key(latest["tag_name"]):
            delete_owned_release(client, release, before_delete=check_replacements)
        elif identity[0] == "beta" and STAGE.fullmatch(release["tag_name"]) and beta is not None:
            delete_owned_release(client, release, before_delete=check_replacements)
            cleanup_stage_tag(client, release["tag_name"], identity[1])
    # A second copy in Actions is unnecessary after both public channels have
    # verified downloads. Active workflow artifacts are deliberately skipped.
    if beta is not None:
        prune_artifacts(client, before_delete=check_replacements)


def stable_notes(config, client):
    if config.notes is not None:
        return config.notes
    try:
        return notes_from_changelog(config.version, ROOT / "CHANGELOG.md")
    except PublishError:
        pass
    previous = [release for release in list_all(client, "/releases")
                if owned_identity(release) and owned_identity(release)[0] == "release"
                and not release.get("draft") and version_key(release["tag_name"]) < version_key(config.version)]
    baseline = max(previous, key=lambda release: version_key(release["tag_name"])) if previous else None
    revision = baseline["tag_name"] + ".." + config.commit if baseline else config.commit
    arguments = ["git", "log", "--format=- %s (%h)", "-n", "201" if baseline else "1", revision]
    try:
        result = subprocess.run(arguments, cwd=ROOT, capture_output=True, text=True, check=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        raise PublishError("Could not generate bounded release notes from the source commits.") from None
    lines = result.splitlines()
    shown, used_bytes = [], 0
    for line in lines[:200]:
        size = len((line + "\n").encode("utf-8"))
        if used_bytes + size > 96 * 1024:
            break
        shown.append(line)
        used_bytes += size
    introduction = "Changes since Solar " + baseline["tag_name"] + ":" if baseline else "Changes in this source commit:"
    fallback = "- Commit subjects exceed the display limit." if lines else "- No source changes."
    notes = introduction + "\n\n" + ("\n".join(shown) or fallback)
    if len(lines) > len(shown):
        notes += f"\n\nShowing the latest {len(shown)} commits within the release-note limit."
    if baseline:
        notes += f"\n\n[Compare changes]({config.server}/{config.repository}/compare/{baseline['tag_name']}...{config.commit})"
    else:
        notes += f"\n\n[Source commit]({config.server}/{config.repository}/commit/{config.commit})"
    return notes


def publish_stable(config, client, files, notes):
    def check_latest():
        if any(owned_identity(release) and owned_identity(release)[0] == "release" and not release.get("draft")
               and version_key(release["tag_name"]) > version_key(config.version)
               for release in list_all(client, "/releases")):
            raise PublishError("A newer stable release is already published; stale publication stopped.")
    check_latest()
    check_tag(client, config.tag, config.commit)
    body = release_body(config, notes)
    release = draft_with_files(config, client, config.tag, body, files)
    require_owned(release, "release", config.commit, config.tag)
    check_tag(client, config.tag, config.commit)
    check_latest()
    if release.get("draft"):
        release = client.request("PATCH", f"/releases/{release['id']}", body={"draft": False})
    require_owned(release, "release", config.commit, config.tag)
    local_assets(client, release, files, complete=True)
    remote_healthy(client, release)
    check_latest()
    retain_latest(client)


def beta_baseline(config, client):
    branch = client.request("GET", "/branches/main")
    if branch.get("commit", {}).get("id") != config.commit:
        raise PublishError("Main advanced during this beta build; current beta was left intact.")
    release = client.request("GET", release_path("beta"), missing_ok=True)
    if release is not None:
        identity = owned_identity(release)
        if identity != ("beta", config.previous_beta) or release.get("draft"):
            raise PublishError("Published beta changed since this build started; stale replacement stopped.")
    return release


def publish_beta(config, client, files, notes):
    # A retry after an unknown PATCH outcome must recognize the already promoted
    # exact files. A manual rebuild of the same SHA may intentionally have new
    # signing timestamps, so matching source identity alone is insufficient.
    existing = client.request("GET", release_path("beta"), missing_ok=True)
    if existing is not None and owned_identity(existing) == ("beta", config.commit) and not existing.get("draft"):
        try:
            local_assets(client, existing, files, complete=True)
        except AssetMismatch:
            if config.previous_beta != config.commit:
                raise PublishError("A different beta for this source is already published; stale retry stopped.") from None
        else:
            remote_healthy(client, existing)
            retain_latest(client)
            return
    baseline = beta_baseline(config, client)
    dmg = next((value for name, value in files.items() if name.endswith(".dmg")))
    stage_tag = f"beta-stage-{config.commit}-{dmg[1][:16]}"
    body = release_body(config, notes)
    stage = draft_with_files(config, client, stage_tag, body, files)
    if stage.get("draft"):
        stage = client.request("PATCH", f"/releases/{stage['id']}", body={"draft": False})
    require_owned(stage, "beta", config.commit, stage_tag)
    local_assets(client, stage, files, complete=True)
    remote_healthy(client, stage)
    # The new public staged download is healthy before removing any old beta
    # assets. Gitea cannot atomically move a tag: the beta URL can briefly be
    # absent during this verified cutover; numeric version tags never move.
    latest = beta_baseline(config, client)
    if (baseline or {}).get("id") != (latest or {}).get("id"):
        raise PublishError("Beta identity changed during staging; replacement stopped.")
    if latest is not None:
        delete_owned_release(client, latest)
    old_tag = client.request("GET", tag_path("beta"), missing_ok=True)
    if old_tag is not None:
        old_commit = old_tag.get("commit", {}).get("sha")
        if old_commit not in (config.previous_beta, config.commit):
            raise PublishError("Rolling beta tag changed unexpectedly; cutover stopped.")
        # A failed PATCH may leave either an indexed tag-only alias or a bare
        # new Git ref. Remove the indexed alias before retrying the DB rename;
        # only the exact new source without an attached release may be reused.
        delete_beta_tag(client, "beta", old_commit,
                        tolerate_unindexed=old_commit == config.commit and latest is None)
    stage = client.request("GET", f"/releases/{stage['id']}")
    require_owned(stage, "beta", config.commit, stage_tag)
    release = client.request("PATCH", f"/releases/{stage['id']}", body={
        "tag_name": "beta", "target_commitish": config.commit, "name": "Solar Beta",
        "body": body, "draft": False, "prerelease": True,
    })
    require_owned(release, "beta", config.commit, "beta")
    local_assets(client, release, files, complete=True)
    remote_healthy(client, release)
    cleanup_stage_tag(client, stage_tag, config.commit)
    retain_latest(client)


def publish(config, client=None):
    files = release_files(config)
    client = client or Gitea(config)
    notes = config.notes if config.channel == "beta" else stable_notes(config, client)
    if not notes or not notes.strip() or len(notes.encode("utf-8")) > 128 * 1024:
        raise PublishError("Nonempty bounded release notes are required.")
    if config.channel == "beta":
        publish_beta(config, client, files, notes)
    else:
        publish_stable(config, client, files, notes)
    return config.server + "/" + config.repository + "/releases/tag/" + config.tag


def main():
    try:
        url = publish(Config.from_env(os.environ))
        print("Verified release published: " + url)
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
                output.write("release_url=" + url + "\n")
    except PublishError as error:
        print("Solar release publication failed: " + str(error), file=sys.stderr)
        return 1
    except (OSError, ValueError, TypeError, KeyError):
        print("Solar release publication failed; draft retained. Check files and API availability.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
