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
import sys
from dataclasses import dataclass, field
import urllib.error
import urllib.parse
import urllib.request

VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\Z")
MAX_ASSET_BYTES = 512 * 1024 * 1024


class PublishError(Exception):
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

    @classmethod
    def from_env(cls, env):
        version = env.get("SOLAR_RELEASE_VERSION", "")
        tags = {env[name] for name in ("RELEASE_TAG", "SOLAR_RELEASE_TAG") if env.get(name)}
        if not VERSION.fullmatch(version) or tags != {version}:
            raise PublishError("Use an exact bare stable version tag matching SOLAR_RELEASE_VERSION.")
        commit = env.get("SOLAR_RELEASE_SHA", env.get("SOLAR_SOURCE_COMMIT", ""))
        if not re.fullmatch(r"(?:[0-9a-f]{40}|[0-9a-f]{64})", commit) or set(commit) == {"0"}:
            raise PublishError("SOLAR_RELEASE_SHA must identify the release commit.")
        repository = env.get("GITEA_REPOSITORY", "")
        if (not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository)
                or any(segment in (".", "..") for segment in repository.split("/"))):
            raise PublishError("GITEA_REPOSITORY must be owner/repository.")
        server = env.get("GITEA_SERVER_URL", "").rstrip("/")
        parsed = urllib.parse.urlsplit(server)
        if (parsed.scheme != "https" or not parsed.hostname or parsed.username
                or parsed.password or parsed.query or parsed.fragment):
            raise PublishError("GITEA_SERVER_URL must be an HTTPS server URL without credentials.")
        token = env.get("GITEA_TOKEN", "")
        if not token or any(character.isspace() for character in token):
            raise PublishError("GITEA_TOKEN is missing or malformed.")
        directory = env.get("SOLAR_RELEASE_DIR", "")
        if not directory or not Path(directory).is_dir():
            raise PublishError("SOLAR_RELEASE_DIR must contain the verified release files.")
        return cls(version, version, Path(directory), server, repository, commit, token)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


class Gitea:
    def __init__(self, config, opener=None):
        self.config = config
        self.opener = opener or urllib.request.build_opener(NoRedirect())
        self.origin = urllib.parse.urlsplit(config.server)[:2]
        self.api = config.server + "/api/v1/repos/" + config.repository

    def request(self, method, path, body=None, upload=None, missing_ok=False):
        headers = {"Authorization": "token " + self.config.token, "Accept": "application/json"}
        data = None
        if upload is not None:
            boundary = "solar-" + secrets.token_hex(24)
            # Filenames are generated locally from validated numeric versions.
            prefix = (f'--{boundary}\r\nContent-Disposition: form-data; name="attachment"; '
                      f'filename="{upload.name}"\r\nContent-Type: application/octet-stream\r\n\r\n').encode()
            data = prefix + upload.read_bytes() + f"\r\n--{boundary}--\r\n".encode()
            headers["Content-Type"] = "multipart/form-data; boundary=" + boundary
        elif body is not None:
            data = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(self.api + path, data=data, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=60) as response:
                data = response.read(4 * 1024 * 1024 + 1)
                if len(data) > 4 * 1024 * 1024:
                    raise PublishError("Release API returned an oversized response.")
                return json.loads(data) if data else None
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            if status == 404 and missing_ok:
                return None
            raise PublishError(f"Gitea release API returned HTTP {status}; draft retained.") from None
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
            raise PublishError("Gitea release API request failed; draft retained for retry.") from None

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
    dmg = config.directory / f"Solar_{config.version}_aarch64.dmg"
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


def publish(config, client=None):
    files = release_files(config)
    notes = config.notes if config.notes is not None else notes_from_changelog(
        config.version, Path(__file__).resolve().parents[2] / "CHANGELOG.md"
    )
    if not notes.strip():
        raise PublishError("Release notes must not be empty.")
    client = client or Gitea(config)
    tag_path = "/tags/" + urllib.parse.quote(config.tag, safe="")
    release_path = "/releases/tags/" + urllib.parse.quote(config.tag, safe="")
    marker = f"<!-- solar-release:v1 tag={config.tag} commit={config.commit} -->"

    def check_tag():
        tag = client.request("GET", tag_path)
        if tag.get("name") != config.tag or tag.get("commit", {}).get("sha") != config.commit:
            raise PublishError("Release tag no longer points to the expected commit.")

    def check_owned(release):
        if (release.get("tag_name") != config.tag or release.get("target_commitish") != config.commit
                or marker not in release.get("body", "") or release.get("prerelease")):
            raise PublishError("Existing release does not match this generated release; left unchanged.")

    def assets(release):
        found = client.request("GET", f"/releases/{release['id']}/assets")
        by_name = {}
        for asset in found:
            name = asset.get("name")
            if name not in files or name in by_name:
                raise PublishError("Release contains unexpected or duplicate assets; left unchanged.")
            path, digest = files[name]
            if asset.get("size") != path.stat().st_size:
                raise PublishError("Existing release asset has different content; left unchanged.")
            remote = client.download_digest(asset.get("browser_download_url", ""), path.stat().st_size)
            if not hmac.compare_digest(remote, digest):
                raise PublishError("Existing release asset checksum differs; left unchanged.")
            by_name[name] = asset
        return by_name

    check_tag()
    release = client.request("GET", release_path, missing_ok=True)
    if release is None:
        release = client.request("POST", "/releases", body={
            "tag_name": config.tag, "target_commitish": config.commit,
            "name": "Solar " + config.version, "draft": True, "prerelease": False,
            "body": f"{notes.strip()}\n\n"
                    "Download the DMG below. Its SHA-256 checksum is supplied in SHA256SUMS.\n\n"
                    f"Source commit: `{config.commit}`\n\n{marker}",
        })
    check_owned(release)
    current = assets(release)
    if not release.get("draft"):
        if set(current) != set(files):
            raise PublishError("Published release is incomplete; it will not be modified.")
        return config.server + "/" + config.repository + "/releases/tag/" + config.tag
    for name, (path, _) in files.items():
        if name in current:
            continue
        client.request("POST", f"/releases/{release['id']}/assets?name="
                       + urllib.parse.quote(name, safe=""), upload=path)
        current = assets(release)
    if set(current) != set(files):
        raise PublishError("Draft is missing release assets; it remains unpublished.")
    # Re-read immediately before publication so a changed tag or edited draft
    # cannot be silently published by a retry.
    check_tag()
    latest = client.request("GET", f"/releases/{release['id']}")
    check_owned(latest)
    if latest.get("draft"):
        latest = client.request("PATCH", f"/releases/{release['id']}", body={"draft": False})
    check_owned(latest)
    if latest.get("draft"):
        raise PublishError("Gitea did not confirm release publication.")
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
