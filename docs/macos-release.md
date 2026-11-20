# Solar releases on GitHub Actions

Source development stays on [Gitea](https://git.ryserve.net/internal/solar-chat); [repository synchronization](github-sync.md) publishes it to [GitHub](https://github.com/rkw6086/solar). All checks and builds run on GitHub. Current installers are on [GitHub Releases](https://github.com/rkw6086/solar/releases).

The release target remains Apple Silicon macOS 14.2+. Native builds use GitHub's `macos-15` hosted runner. This migration does not add Windows, Linux, Intel, or universal installers.

## Stable and beta channels

- A new bare Semantic Version tag, such as `0.1.4`, builds its exact source as a stable release.
- A schedule checks `main` at **3:00 a.m. America/Chicago**, including daylight saving time. An unchanged, complete beta is skipped.
- Manually running **Build Solar** on `main` always rebuilds the beta.
- Pull requests build an unsigned app without signing credentials or publication access.
- **Solar checks** validates frontend builds, tests, and release scripts on branch pushes and pull requests.

Stable and beta publication share one queue. Each build pins its source commit before signing. A staged replacement is downloaded and verified before the previous release is removed. Exactly one owned stable release and one rolling `beta` release are retained; numbered source tags remain immutable. Missing or unrecognized release data fails closed rather than deleting it.

The signed DMG is `Solar_VERSION_aarch64.dmg` for stable releases and `Solar_beta_FULL_COMMIT_aarch64.dmg` for beta. Every release includes `SHA256SUMS`. About Solar shows the packaged version; beta also shows its source commit. Both channels use the same app identifier and `~/.solar` data.

## Publish a stable version

From the primary Gitea checkout:

```sh
corepack pnpm run release:prepare patch
# Add the version's notes to CHANGELOG.md, then commit and push main to origin.
corepack pnpm run version:check
corepack pnpm run release:tag VERSION --push
```

Replace `VERSION` with the prepared version. Use `minor`, `major`, or an exact version instead of `patch` when appropriate. Do not recreate or move an existing numbered tag. Gitea sync delivers the new tag to GitHub, which triggers the signed build. A failed build can be rerun for the same unchanged tag.

## Signing and notarization

Use an Apple Developer Program **Developer ID Application** certificate with its matching private key. A Developer ID Installer certificate is not required for a DMG. The same team certificate can sign Solar and Galaxy; their bundle identifiers remain distinct.

Create a CSR in Keychain Access, obtain the certificate from Apple's developer portal, and export the certificate together with its private key as an encrypted `.p12`. Keep a private backup. An App Store Connect **team API key** authenticates notarization; an individual key is not suitable for this workflow. Save its Key ID, Issuer ID, and downloaded `.p8` private key.

Configure these as **GitHub Actions repository secrets** on `rkw6086/solar`:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the encrypted `.p12`, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: … (TEAMID)` identity |
| `APPLE_TEAM_ID` | Apple Developer Team ID |
| `APPLE_API_KEY` | App Store Connect team Key ID |
| `APPLE_API_ISSUER` | App Store Connect Issuer ID |
| `APPLE_API_KEY_P8` | Base64 of the team API key's `.p8` |

The configured identity and API key are shared with Galaxy, but GitHub repository secrets are separate copies. Rotate both repositories' copies together. No signing key belongs in source, issue text, logs, or build artifacts.

The unsigned build runs without Apple credentials. Packaging imports the certificate into an ephemeral keychain, signs nested code and the app, notarizes and staples the app, creates and signs the DMG, and notarizes and staples the DMG. It checks hardened runtime, team identity, signatures, notarization, and Gatekeeper before upload. Cleanup restores the keychain search list and removes the temporary keychain and credential files even on failure.

Unsigned compiler entries and public dependency downloads are cached separately from artifacts and signing material. Only trusted `main` scheduled/manual builds save caches. Each transfer artifact expires after one day and is removed after successful publication. Final apps, signed packages, private configuration, and credentials are never part of the compiler cache.

## Verify a release

Wait for the entire **Build Solar** run to pass. Download the DMG and `SHA256SUMS` from the matching GitHub release and verify:

```sh
shasum -a 256 -c SHA256SUMS
```

Mount the image on a Mac, install Solar, and check launch, screen sharing, Accessibility permissions, popup chats, and widget motion. Signing and notarization do not replace functional testing. Confirm the beta points to the intended source, the stable release is unchanged during a beta update, superseded downloads are gone, and only the two intended channels remain.

Earlier Gitea workflow definitions are kept under `.gitea/retired-workflows/` for reference; they no longer run. Previous Gitea downloads are legacy records, not the destination for new releases.
