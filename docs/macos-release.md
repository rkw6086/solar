# Signed macOS releases with Gitea Actions

Solar's release workflow is [`.gitea/workflows/macos-release.yml`](../.gitea/workflows/macos-release.yml). [`scripts/ci/build-macos.sh`](../scripts/ci/build-macos.sh) builds and checks the app; [`scripts/ci/package-macos.sh`](../scripts/ci/package-macos.sh) signs and packages it.

The stable workflow targets the existing **`macos-arm64`** runner and produces an Apple Silicon DMG when a new version tag is pushed. It builds Solar and the bundled Swift helper, signs the helper before the app, notarizes and staples the app, creates a DMG containing `Solar.app` and an Applications shortcut, then signs, notarizes, staples, and verifies the DMG. After those checks succeed, it publishes the DMG and `SHA256SUMS` on [Gitea Releases](https://git.ryserve.net/internal/solar-chat/releases), using that version's changelog notes.

## Stable and nightly beta channels

Solar keeps **one published stable release and one rolling beta release**. A stable release uses its numbered tag, such as `0.1.3`. The [beta workflow](../.gitea/workflows/macos-beta.yml) checks current `main` at **3:00 a.m. America/Chicago** with `CRON_TZ=America/Chicago 0 3 * * *`, including daylight saving time. It skips an unchanged, complete beta; start the same workflow manually on `main` to rebuild on demand. Branch pushes and pull requests do not sign builds.

The workflow checks out live `main` and validates it against Gitea before building. The rolling [`beta` release](https://git.ryserve.net/internal/solar-chat/releases/tag/beta) contains `Solar_beta_COMMIT_aarch64.dmg` and `SHA256SUMS`; About Solar shows its package version, beta channel, and source commit. Beta release notes describe commits since the previous successfully published beta, not failed build attempts.

Stable and beta build, sign, publish, and prune within the same capacity-one native runner job. There is no duplicate Actions artifact upload. A new download is checked remotely before older release records and assets are removed. The beta uses a verified staging release before replacing its rolling tag; Gitea cannot switch tag and release atomically, so the beta URL may be briefly unavailable during cutover while the staging download remains recoverable. An uncertain API write stops cleanup; retrying reconciles the exact owned state.

Historical numbered **Git tags remain immutable** for source history; their old release pages and DMG/checksum downloads are removed after a newer stable download verifies. Old owned Solar macOS Actions archives from completed runs are removed once both current channels verify healthy. Active runs, unrelated artifacts/releases, and shared runner caches are preserved. Gitea reclaims deleted archive bytes asynchronously. Tiny asset-free staging refs may remain if Gitea cannot delete an unindexed ref through its tag API.

Stable and beta use the same application identifier and `~/.solar` profile. Installing the stable DMG restores the stable application without moving profile data.

The files alone do not configure Apple signing. Complete the certificate, API-key, and Gitea-secret steps below before running the workflow.

## 1. Prepare the runner

Register the runner label as **`macos-arm64:host`**. The workflow uses `runs-on: macos-arm64`; the `:host` suffix belongs in the runner configuration, not in the workflow. It must run directly on an Apple Silicon Mac, with a native arm64 process, rather than a Linux container. [Gitea runner labels](https://docs.gitea.com/runner/labels/)

Install or make available to the runner's service account:

- macOS 14.2 or newer and Xcode with the command-line tools selected, its license accepted, and a compatible macOS SDK/Swift compiler. Solar's native helper uses AppKit, ScreenCaptureKit, and ApplicationServices.
- Git, Bash, `curl`, Python 3, and Node.js on `PATH`. JavaScript Actions need a working Node runtime before the workflow's Node setup step runs.
- Rust through `rustup`. The workflow selects Rust **1.99.1** and target `aarch64-apple-darwin`. Its frontend tooling uses Node **24.17.0**, Corepack **0.36.0**, and the pnpm version pinned in `package.json`.
- Outbound access to the Gitea server, the pinned Actions sources, dependency registries, and Apple's notarization services.

Check under the **same user account that runs the runner**, not only in your interactive shell:

```sh
uname -m
xcode-select -p
xcrun --find swiftc
xcrun --find notarytool
xcrun --find stapler
node --version
rustup --version
```

`uname -m` should report `arm64`. The workflow installs the frontend dependencies using the `packageManager` version in `package.json` and the committed lockfile.

Use a dedicated trusted-repository runner with **capacity 1**. The release script temporarily changes the runner user's keychain search list and restores it afterward; do not run another signing job under that user concurrently. Host jobs execute as that user without container isolation. Restrict this runner to trusted repositories and protect the release branch and tags. [Gitea runner security model](https://docs.gitea.com/runner/)

## 2. Create the Developer ID Application certificate

Use the Apple Developer Program **Account Holder** account to create this certificate. A **Developer ID Application** certificate signs apps distributed outside the Mac App Store. A Developer ID Installer certificate is for installer packages and is not needed for Solar's DMG. [Apple's certificate instructions](https://developer.apple.com/help/account/certificates/create-developer-id-certificates/)

A valid Developer ID Application certificate and its matching private key can sign multiple macOS apps from the same Apple Developer team. Solar and Galaxy share this signing identity; a new certificate is not needed for each app. Their app bundle identifiers remain separate.

On your Mac:

1. Open **Keychain Access**.
2. Choose **Keychain Access → Certificate Assistant → Request a Certificate From a Certificate Authority**.
3. Enter your email address and a recognizable common name, leave the CA email blank, select **Saved to disk**, and save the `.certSigningRequest` file. This creates the matching private key in your local keychain. [Apple's CSR instructions](https://developer.apple.com/help/account/certificates/create-a-certificate-signing-request/)
4. Open [Certificates, Identifiers & Profiles](https://developer.apple.com/account/resources/certificates/list), click **+**, select **Developer ID Application**, and upload the CSR.
5. Download the `.cer` and open it on the same Mac where you created the CSR.
6. In Keychain Access, select **login → My Certificates**. Confirm the certificate has an expandable **private key** beneath it. A certificate without that private key cannot sign releases.
7. Export the certificate together with its private key as a `.p12`, and set a strong export password. Keep a secure backup.
8. Copy the full signing identity from:

```sh
security find-identity -v -p codesigning
```

It should look like `Developer ID Application: Your Name or Company (TEAMID)`. Use the full quoted identity text, without the surrounding quotes, as `APPLE_SIGNING_IDENTITY`. The value in parentheses is the `APPLE_TEAM_ID`; you can also find it in your Apple Developer account's membership details.

Copy the `.p12` as base64 to your clipboard without printing it in terminal output:

```sh
/usr/bin/base64 -i /path/to/Solar-Developer-ID.p12 | pbcopy
```

Paste that into the `APPLE_CERTIFICATE` Gitea secret described below. Base64 is an encoding, not encryption; treat it as the private key. Do not paste it into chat, an issue, a commit, or a workflow file. [Tauri's CI certificate export guidance](https://v2.tauri.app/distribute/sign/macos/)

## 3. Create an App Store Connect team API key

This is separate from the signing certificate. It authenticates the upload to Apple's notarization service; it does not sign the app.

1. Open [App Store Connect](https://appstoreconnect.apple.com/), then **Users and Access → Integrations → App Store Connect API**.
2. Create a **team API key** named, for example, `Solar notarization`, with **Developer** access. An Account Holder or Admin may need to enable API access or create the key.
3. Save its **Key ID** and **Issuer ID**.
4. Download the `.p8` private key. Apple permits this download only once, so keep a secure backup.

Use a **team key**, not an individual key: individual keys do not support `notaryTool`. Team keys are not limited to one app, so protect the key accordingly. [Apple's API-key guidance](https://developer.apple.com/documentation/appstoreconnectapi/creating-api-keys-for-app-store-connect-api)

```sh
/usr/bin/base64 -i /path/to/AuthKey_KEYID.p8 | pbcopy
```

Paste this into `APPLE_API_KEY_P8`. No Apple account password is required by this workflow.

## 4. Configure shared organization secrets

All seven `APPLE_*` secrets below are stored as **organization Actions secrets** in both Gitea organizations:

- **`internal`** supplies the secrets to `internal/solar-chat` (Solar).
- **`public`** supplies the same secrets to `public/galaxy` (Galaxy).

Gitea does not support an instance-global Actions secret store. Organization secrets are inherited by repositories owned by that organization, so both organizations need the same values. In each organization's **Settings → Actions → Secrets**, add or update:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the exported `.p12`, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: … (TEAMID)` identity |
| `APPLE_TEAM_ID` | The Apple Developer team's ID |
| `APPLE_API_KEY` | App Store Connect team API Key ID |
| `APPLE_API_ISSUER` | App Store Connect Issuer ID |
| `APPLE_API_KEY_P8` | Base64 of the downloaded `.p8` private key |

Keep these names identical in both organizations. Workflows reference them as `${{ secrets.APPLE_CERTIFICATE }}` and so on; no repository-specific copies are needed. Rotate the values in both organizations together.

Remove any repository-level overrides for these seven names after confirming the organization values are configured. A repository secret with the same name takes precedence and can silently keep using an old certificate or key. Organization secrets are available to every repository owned by that organization; only trusted workflows should receive signing credentials. [Gitea Actions secrets](https://docs.gitea.com/usage/actions/secrets/), [Gitea secret inheritance and precedence](https://github.com/go-gitea/gitea/blob/v1.27.3/models/secret/secret.go)

Each repository must have **Actions enabled** and a signing workflow that references these secret names. Sharing the credentials makes them available to Galaxy; it does not create Galaxy's release workflow.

The packaging script creates and registers a temporary keychain under the runner user's `~/Library/Keychains` before importing the certificate and Apple's public Developer ID G2 intermediate. It verifies both basic and code-signing certificate chains against macOS's existing trusted roots before signing; it does not change certificate trust settings. It restores the original keychain search lists, deletes the temporary keychain, and removes temporary credential files on exit. The preceding build step receives no signing secrets and uses Tauri's `--no-sign` option; packaging then signs the helper and app explicitly. Do not enable shell tracing (`set -x`) around credential handling. Protect `main` and version tags so unreviewed changes cannot run with signing credentials.

The publishing step uses Gitea's built-in Actions token with release-write permission. It does not require another personal access token or another Apple secret.

The Mac publishes its verified files directly to Gitea Releases before releasing its runner slot. Publication and retention use release-write and Actions-write permissions; the beta additionally needs code-write permission for its rolling and staging tags. Credentials remain limited to their required steps. A failed publication preserves recoverable release state instead of deleting working downloads.

## 5. Prepare and publish a version

The Git tag is the release version: **`0.1.0`**, not `v0.1.0`. Use stable Semantic Versioning in the form `MAJOR.MINOR.PATCH`. Prerelease suffixes and build metadata are not currently supported, so the same values also fit Apple's app-version requirements. **About Solar** displays the version packaged into the app.

All Rust crates inherit the workspace version. The release helper keeps `Cargo.toml`, `package.json`, `src-tauri/tauri.conf.json`, and the four local packages in `Cargo.lock` synchronized. Check them at any time with:

```sh
corepack pnpm run version:check
```

For the next release, choose `patch` for fixes, `minor` for features, or `major` for breaking changes. You can also supply an exact version:

```sh
corepack pnpm run release:prepare patch
# Alternatives: release:prepare minor, release:prepare major, release:prepare 0.2.0
```

This prepares version files; it does not commit or publish anything. Add release notes to `CHANGELOG.md` under an exact heading such as `## [0.1.1]`, run the checks, and commit and push the changes to `main`. Then create and push the matching annotated tag. This example publishes `0.1.1`; replace it with the version you prepared:

```sh
corepack pnpm run version:check
git add Cargo.toml Cargo.lock package.json src-tauri/tauri.conf.json CHANGELOG.md
git commit -m "release: 0.1.1"
git push origin main
corepack pnpm run release:tag 0.1.1 --push
```

For the initial release, use `0.1.0` with matching version files and changelog notes. Without `--push`, `release:tag` creates the tag locally and prints the command to push it. The helper checks the version, notes, and Git state before tagging.

Only a newly pushed numbered tag starts stable signing. Nightly/manual beta signing is restricted to `main`. Branch pushes and pull requests run release checks without Apple credentials. CI rejects mismatched, moved, reused, and stale numbered tags; source changes for a stable release belong in a new version. Protect numbered tags and `main`, while allowing the trusted beta workflow to replace its owned `beta` and staging refs.

The job verifies the app's code signature, notarization ticket, and Gatekeeper assessment, then does the corresponding checks for the DMG. Each Apple submission waits for up to 90 minutes; Apple's first notarization submission may take longer than later submissions. Failure leaves the job failed; an unsigned or rejected package is not presented as a successful release. [Apple notarization overview](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution/)

Download the DMG and `SHA256SUMS` from the version's [Gitea Release](https://git.ryserve.net/internal/solar-chat/releases). Build and packaging files live in job-specific temporary directories and are removed after publication. New builds do not retain duplicate Actions archives. Test the downloaded DMG on a Mac: mount it, drag Solar to Applications, launch it, and exercise screen sharing and Accessibility permissions. Signing/notarization checks do not replace that functional test. [Gitea artifacts](https://docs.gitea.com/usage/actions/artifacts/)

This initial workflow builds Apple Silicon only. Solar's helper staging currently supports separate `aarch64-apple-darwin` and `x86_64-apple-darwin` builds, not a universal helper; an Intel or universal release needs a separate extension to the pipeline.
