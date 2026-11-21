#!/usr/bin/env python3
"""Publish verified GitHub downloads, retaining one stable release and one beta.

The replacement/retention state machine is shared with Solar's proven Gitea
flow; the GitHub transport follows Galaxy's authenticated draft discovery,
asset API downloads and lease-protected rolling tag operations.
"""
from __future__ import annotations

import hashlib
import hmac
import json
import os
from pathlib import Path
import re
import secrets
import tempfile
import subprocess
import sys
import time
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
    previous_beta_release_id: int | None = None

    @classmethod
    def from_env(cls, env):
        version = env.get("SOLAR_RELEASE_VERSION", "")
        channel = env.get("SOLAR_BUILD_CHANNEL", "")
        commit = env.get("SOLAR_RELEASE_SHA", "")
        if channel not in ("release", "beta") or not VERSION.fullmatch(version) or not COMMIT.fullmatch(commit) or set(commit) == {"0"}:
            raise PublishError("Publication requires a validated stable or beta source.")
        if (env.get("GITHUB_SERVER_URL") != "https://github.com"
                or env.get("GITHUB_REPOSITORY") != "rkw6086/solar"
                or env.get("GITHUB_API_URL", "https://api.github.com") != "https://api.github.com"):
            raise PublishError("Publication is restricted to Solar on GitHub.")
        event, ref = env.get("GITHUB_EVENT_NAME"), env.get("GITHUB_REF")
        if channel == "release" and (event != "push" or ref != "refs/tags/" + version):
            raise PublishError("Stable publication requires its pushed version tag.")
        if channel == "beta" and (event not in ("schedule", "workflow_dispatch") or ref != "refs/heads/main"):
            raise PublishError("Beta publication requires a scheduled or manual main run.")
        if channel == "beta" and env.get("SOLAR_BETA_ID") != commit:
            raise PublishError("Beta identity does not match its validated source.")
        token = env.get("GITHUB_TOKEN", "")
        if not token or any(character.isspace() for character in token):
            raise PublishError("GITHUB_TOKEN is missing or malformed.")
        directory = Path(env.get("SOLAR_RELEASE_DIR", ""))
        if not directory.is_absolute() or not directory.is_dir() or directory.is_symlink():
            raise PublishError("SOLAR_RELEASE_DIR must be an absolute regular artifact directory.")
        notes = None
        if env.get("SOLAR_RELEASE_NOTES_FILE"):
            path = Path(env["SOLAR_RELEASE_NOTES_FILE"])
            if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= 128 * 1024:
                raise PublishError("Release notes must be a bounded regular file.")
            notes = path.read_text(encoding="utf-8")
        previous = env.get("SOLAR_PREVIOUS_BETA_SHA") or None
        previous_id = env.get("SOLAR_PREVIOUS_BETA_RELEASE_ID") or None
        if previous and not COMMIT.fullmatch(previous):
            raise PublishError("Previous beta identity must be an exact commit.")
        if bool(previous) != bool(previous_id) or (previous_id and not re.fullmatch(r"[1-9][0-9]{0,18}", previous_id)):
            raise PublishError("Previous beta needs its exact source and release identity.")
        return cls(version, "beta" if channel == "beta" else version, directory,
                   "https://github.com", "rkw6086/solar", commit, token, notes, channel,
                   previous, int(previous_id) if previous_id else None)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class GitHub:
    def __init__(self, config, opener=None):
        self.config = config
        self.opener = opener or urllib.request.build_opener(NoRedirect())
        self.api = "https://api.github.com/repos/" + config.repository
        self.verified_asset_digests = {}
        self.asset_ids = {}

    def api_request(self, method, path, body=None, upload=None, missing_ok=False):
        headers = {"Authorization": "Bearer " + self.config.token,
                   "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2026-03-10"}
        if method == "GET":
            # Release/retention decisions must not reuse an intermediary's
            # cached response after a successful mutation.
            headers.update({"Cache-Control": "no-cache", "Pragma": "no-cache"})
        url, data, stream = self.api + path, None, None
        if upload is not None:
            if method != "POST" or not re.fullmatch(r"/releases/[1-9][0-9]*/assets\?name=[A-Za-z0-9_.%-]+", path):
                raise PublishError("Invalid release asset upload identity.")
            url = "https://uploads.github.com/repos/" + self.config.repository + path
            stream = upload.open("rb")
            data = stream
            headers.update({"Content-Type": "application/octet-stream", "Content-Length": str(upload.stat().st_size)})
        elif body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        try:
            with self.opener.open(urllib.request.Request(url, data=data, headers=headers, method=method),
                                  timeout=300 if stream else 60) as response:
                data = response.read(8 * 1024 * 1024 + 1)
                if len(data) > 8 * 1024 * 1024:
                    raise PublishError("Release API returned an oversized response.")
                return json.loads(data) if data else None
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            if status == 404 and missing_ok:
                return None
            if status == 404:
                raise MissingResource("GitHub release API resource was not found.") from None
            raise PublishError(f"GitHub release API returned HTTP {status}; reconcile state before retrying a write.") from None
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
            raise PublishError("GitHub API operation failed; a write may have completed. Reconcile retained state before retrying.") from None
        finally:
            if stream is not None:
                stream.close()

    def tag(self, name, missing_ok=False):
        value = self.api_request("GET", "/git/ref/tags/" + urllib.parse.quote(name, safe=""), missing_ok=missing_ok)
        if value is None:
            return None
        if value.get("ref") != "refs/tags/" + name:
            raise PublishError("Unexpected GitHub tag identity.")
        obj = value.get("object", {})
        for _ in range(8):
            if obj.get("type") == "commit" and COMMIT.fullmatch(obj.get("sha", "")):
                return {"name": name, "commit": {"sha": obj["sha"]}, "object": value["object"]}
            if obj.get("type") != "tag" or not COMMIT.fullmatch(obj.get("sha", "")):
                break
            obj = self.api_request("GET", "/git/tags/" + obj["sha"]).get("object", {})
        raise PublishError("GitHub tag does not identify a bounded commit object.")

    def lease_tag(self, name, expected, target):
        if name != "beta" and not STAGE.fullmatch(name):
            raise PublishError("Only beta and its exact staging aliases can be changed.")
        if expected and not COMMIT.fullmatch(expected):
            raise PublishError("Invalid rolling tag lease.")
        if target and target != self.config.commit:
            raise PublishError("Rolling aliases can only point to this verified source.")
        with tempfile.TemporaryDirectory(prefix="solar-beta-auth-") as directory:
            askpass = Path(directory) / "askpass.py"
            askpass.write_text("#!/usr/bin/env python3\nimport os,sys\np=sys.argv[1].lower() if len(sys.argv)>1 else ''\n"
                "if 'github.com' not in p: sys.exit(1)\n"
                "if 'username' in p: print('x-access-token')\n"
                "elif 'password' in p: print(os.environ['SOLAR_PUBLISH_TOKEN'])\n"
                "else: sys.exit(1)\n")
            askpass.chmod(0o700)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith("GIT_") and key not in ("GH_TOKEN", "GITHUB_TOKEN", "SSH_ASKPASS", "SSH_ASKPASS_REQUIRE")}
            env.update(GIT_ASKPASS=str(askpass), GIT_TERMINAL_PROMPT="0", GIT_CONFIG_GLOBAL=os.devnull,
                       GIT_CONFIG_NOSYSTEM="1", SOLAR_PUBLISH_TOKEN=self.config.token)
            try:
                result = subprocess.run(["git", "-c", "credential.helper=", "-c", "core.hooksPath=" + os.devnull,
                    "-c", "http.followRedirects=false", "push", "--no-follow-tags",
                    "--force-with-lease=refs/tags/" + name + ":" + (expected or ""),
                    "https://github.com/" + self.config.repository + ".git", (target or "") + ":refs/tags/" + name],
                    cwd=ROOT, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                    timeout=120, check=False)
            except (OSError, subprocess.SubprocessError):
                raise PublishError("Rolling tag update outcome is unknown; reconcile state before retrying.") from None
            if result.returncode:
                raise PublishError("Rolling tag lease was rejected or transport failed; no other refs were changed.")
        current = self.tag(name, missing_ok=True)
        if (current or {}).get("commit", {}).get("sha") != target:
            raise PublishError("Rolling tag update was not confirmed.")

    def release_for_tag(self, name, missing_ok=False):
        # GitHub's by-tag endpoint omits drafts. List using the privileged
        # publication token, then re-read the exact record before acting on it.
        matches = [value for value in list_all(self, "/releases") if value.get("tag_name") == name]
        if len(matches) > 1:
            raise PublishError("Multiple releases claim the same tag; inspect them before continuing.")
        if not matches:
            if missing_ok:
                return None
            raise MissingResource("GitHub release was not found.")
        result = self.api_request("GET", f"/releases/{matches[0]['id']}")
        if result.get("tag_name") != name or result.get("id") != matches[0]["id"]:
            raise PublishError("Release changed during authenticated lookup.")
        return result

    def request(self, method, path, body=None, upload=None, missing_ok=False):
        route = urllib.parse.urlsplit(path)
        if route.path.startswith("/tags/"):
            name = urllib.parse.unquote(route.path[len("/tags/"):])
            if method == "GET":
                return self.tag(name, missing_ok)
            if method == "DELETE":
                if name != "beta" and not STAGE.fullmatch(name):
                    raise PublishError("Only beta and its owned staging aliases may be removed.")
                current = self.tag(name, missing_ok=True)
                if current is not None:
                    obj = current.get("object", {})
                    if obj.get("type") != "commit":
                        raise PublishError("Rolling aliases must use lightweight source tags.")
                    expected = (self.config.previous_beta, self.config.commit) if name == "beta" else (STAGE.fullmatch(name)[1],)
                    if obj["sha"] not in expected:
                        raise PublishError("Rolling alias changed before deletion; left unchanged.")
                    self.lease_tag(name, obj["sha"], None)
                return None
            raise PublishError("Unsupported tag operation.")
        if route.path.startswith("/releases/tags/") and method == "GET":
            return self.release_for_tag(urllib.parse.unquote(route.path[len("/releases/tags/"):]), missing_ok)
        # Materialize each staging ref before making or updating a draft so
        # GitHub never changes it into an internal untagged-* placeholder.
        if (method == "POST" and route.path == "/releases") or (method == "PATCH" and body and "tag_name" in body):
            name = body["tag_name"]
            current = self.tag(name, missing_ok=True)
            if current is None:
                self.lease_tag(name, None, body["target_commitish"])
            elif current["commit"]["sha"] != body["target_commitish"]:
                raise PublishError("Release tag differs from the staged source.")
        params = urllib.parse.parse_qsl(route.query)
        params = [("per_page" if key == "limit" else key, value) for key, value in params]
        path = route.path + ("?" + urllib.parse.urlencode(params) if params else "")
        result = self.api_request(method, path, body, upload, missing_ok)
        if method == "GET" and route.path == "/branches/main" and result:
            result["commit"]["id"] = result["commit"]["sha"]
        if method == "GET" and re.fullmatch(r"/releases/[1-9][0-9]*/assets", route.path) and isinstance(result, list):
            for asset in result:
                self.asset_ids[asset["browser_download_url"]] = asset["id"]
        return result

    def download(self, url, limit, expected_size=None, digest=False):
        asset_id = self.asset_ids.get(url)
        if not isinstance(asset_id, int) or asset_id <= 0:
            raise PublishError("Download must match freshly read GitHub asset metadata.")
        url = self.api + f"/releases/assets/{asset_id}"
        for _ in range(6):
            parsed = urllib.parse.urlsplit(url)
            if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
                raise PublishError("Release download is not a safe HTTPS URL.")
            headers = {"Accept": "application/octet-stream"}
            if parsed.netloc == "api.github.com":
                headers.update(Authorization="Bearer " + self.config.token)
                headers["X-GitHub-Api-Version"] = "2026-03-10"
            try:
                with self.opener.open(urllib.request.Request(url, headers=headers), timeout=180) as response:
                    value = hashlib.sha256() if digest else bytearray()
                    size = 0
                    while chunk := response.read(1024 * 1024):
                        size += len(chunk)
                        if size > limit or (expected_size is not None and size > expected_size):
                            raise PublishError("Release download exceeded its declared bounds.")
                        value.update(chunk) if digest else value.extend(chunk)
                    if expected_size is not None and size != expected_size:
                        raise PublishError("Release asset download size did not match metadata.")
                    return value.hexdigest() if digest else bytes(value)
            except urllib.error.HTTPError as error:
                status, location = error.code, error.headers.get("Location")
                error.close()
                if status in (301, 302, 303, 307, 308) and location:
                    url = urllib.parse.urljoin(url, location)
                    continue
                raise PublishError(f"Release asset download returned HTTP {status}; URLs and credentials withheld.") from None
            except (urllib.error.URLError, TimeoutError):
                raise PublishError("Release download failed; URLs and credentials withheld.") from None
        raise PublishError("Release download exceeded its redirect limit.")

    def download_bytes(self, url, limit):
        return self.download(url, limit)

    def download_digest(self, url, expected_size):
        return self.download(url, MAX_ASSET_BYTES, expected_size, digest=True)


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
    # GitHub attachment IDs identify immutable file content. Renaming/promoting
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
    # GitHub's release endpoint removes attachments, but retains the Git tag.
    client.request("DELETE", f"/releases/{release['id']}")
    if client.request("GET", f"/releases/{release['id']}", missing_ok=True) is not None:
        raise PublishError("Release deletion was not confirmed.")
    if before_tag:
        check_tag(client, release["tag_name"], commit)


def delete_beta_tag(client, tag, commit):
    if tag != "beta" and not STAGE.fullmatch(tag):
        raise PublishError("Only beta and its staging aliases may be removed.")
    value = client.request("GET", tag_path(tag), missing_ok=True)
    if value is None:
        return
    check_tag(client, tag, commit)
    if client.request("GET", release_path(tag), missing_ok=True) is not None:
        raise PublishError("Cannot remove a tag still attached to a release.")
    client.request("DELETE", tag_path(tag))
    if client.request("GET", tag_path(tag), missing_ok=True) is not None:
        raise PublishError("Beta tag deletion was not confirmed.")


def cleanup_stage_tag(client, tag, commit):
    try:
        delete_beta_tag(client, tag, commit)
    except PublishError:
        print("Internal beta staging ref cleanup deferred; verified public downloads remain available.")


def confirm_artifact_deleted(client, artifact):
    # GitHub's artifact metadata can briefly lag a successful DELETE. Poll
    # only fresh GETs, never repeat the mutation or silently accept a failure.
    for delay in (0, 0.5, 1, 2, 4, 8):
        if delay:
            time.sleep(delay)
        path = f"/actions/artifacts/{artifact['id']}?solar_delete_check={secrets.token_hex(8)}"
        remaining = client.request("GET", path, missing_ok=True)
        if remaining is None:
            return
        if (remaining.get("id") != artifact["id"]
                or remaining.get("name") != artifact["name"]
                or remaining.get("workflow_run", {}).get("id") != artifact.get("workflow_run", {}).get("id")):
            raise PublishError("Actions artifact identity changed while confirming deletion; no further writes made.")
        if remaining.get("expired"):
            return
    raise PublishError("Actions artifact deletion was not confirmed after bounded fresh checks.")


def prune_artifacts(client, before_delete=None):
    # Never touch PR outputs or artifacts from another workflow. The one-day
    # upload retention is the fallback if publication or cleanup is interrupted.
    known = {"solar-macos", "solar-release-notes"}
    for artifact in list_all(client, "/actions/artifacts", "artifacts"):
        if artifact.get("name") not in known or artifact.get("expired"):
            continue
        run_id = artifact.get("workflow_run", {}).get("id")
        if not isinstance(run_id, int) or run_id <= 0:
            continue
        run = client.request("GET", f"/actions/runs/{run_id}")
        own_active = str(run_id) == os.environ.get("GITHUB_RUN_ID")
        if (run.get("path") != ".github/workflows/build.yml"
                or run.get("event") not in ("push", "schedule", "workflow_dispatch")
                or (run.get("status") != "completed" and not own_active)):
            continue
        current = client.request("GET", f"/actions/artifacts/{artifact['id']}", missing_ok=True)
        if current is None or current.get("expired"):
            continue
        if (current.get("id") != artifact["id"] or current.get("name") != artifact["name"]
                or current.get("workflow_run", {}).get("id") != run_id):
            raise PublishError("Actions artifact changed before cleanup.")
        if before_delete:
            before_delete()
        client.request("DELETE", f"/actions/artifacts/{artifact['id']}")
        confirm_artifact_deleted(client, current)


def retain_latest(client):
    releases = list_all(client, "/releases")
    stable = [release for release in releases if owned_identity(release)
              and owned_identity(release)[0] == "release" and not release.get("draft")]
    latest = max(stable, key=lambda release: version_key(release["tag_name"])) if stable else None
    if latest is not None:
        remote_healthy(client, latest)
    beta = client.request("GET", release_path("beta"), missing_ok=True)
    if beta is not None:
        remote_healthy(client, beta)
    def check_replacements():
        # A manual edit/deletion outside the serialized native workflow must
        # stop pruning rather than leave the user without verified downloads.
        if latest is not None:
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
        if latest is not None and identity[0] == "release" and version_key(release["tag_name"]) < version_key(latest["tag_name"]):
            delete_owned_release(client, release, before_delete=check_replacements)
        elif identity[0] == "beta" and STAGE.fullmatch(release["tag_name"]) and beta is not None:
            delete_owned_release(client, release, before_delete=check_replacements)
            cleanup_stage_tag(client, release["tag_name"], identity[1])
    # A second copy in Actions is unnecessary after this published candidate
    # and any retained channel have verified downloads.
    if beta is not None or latest is not None:
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
        if (identity != ("beta", config.previous_beta) or release.get("draft")
                or (config.previous_beta_release_id is not None and release.get("id") != config.previous_beta_release_id)):
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
    # Bind the prior alias to this verified candidate so a later workflow can
    # recognize an interrupted cutover without treating an orphan ref as owned.
    stage_body = (body + f"\n\n<!-- solar-beta-transition:v1 previous={config.previous_beta or 'none'} "
                  + f"release={config.previous_beta_release_id or 'none'} -->")
    stage = draft_with_files(config, client, stage_tag, stage_body, files)
    if stage.get("draft"):
        stage = client.request("PATCH", f"/releases/{stage['id']}", body={"draft": False})
    require_owned(stage, "beta", config.commit, stage_tag)
    local_assets(client, stage, files, complete=True)
    remote_healthy(client, stage)
    # The new public staged download is healthy before removing any old beta
    # assets. The beta URL can briefly be
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
        # A failed promotion may leave the exact new alias without its release.
        # Only the captured prior source or this new candidate may be removed.
        delete_beta_tag(client, "beta", old_commit)
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
    client = client or GitHub(config)
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
