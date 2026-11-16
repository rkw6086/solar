# Signed macOS releases with Gitea Actions

Solar's release workflow is [`.gitea/workflows/macos-release.yml`](../.gitea/workflows/macos-release.yml). [`scripts/ci/build-macos.sh`](../scripts/ci/build-macos.sh) builds and checks the app; [`scripts/ci/package-macos.sh`](../scripts/ci/package-macos.sh) signs and packages it.

The workflow targets the existing **`macos-arm64`** runner and produces an Apple Silicon DMG. It builds Solar and the bundled Swift helper, signs the helper before the app, notarizes and staples the app, creates a DMG containing `Solar.app` and an Applications shortcut, then signs, notarizes, staples, and verifies the DMG. The output is an Actions artifact; the workflow does not create a public release or publish a download automatically.

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

## 4. Add repository secrets

In [Solar's repository settings](https://git.ryserve.net/internal/solar-chat/settings), open **Actions → Secrets** and add:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64 of the exported `.p12`, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: … (TEAMID)` identity |
| `APPLE_TEAM_ID` | The Apple Developer team's ID |
| `APPLE_API_KEY` | App Store Connect team API Key ID |
| `APPLE_API_ISSUER` | App Store Connect Issuer ID |
| `APPLE_API_KEY_P8` | Base64 of the downloaded `.p8` private key |

The repository must have **Actions enabled**. Keep secrets at repository scope unless you deliberately intend other repositories to use them. [Gitea Actions secrets](https://docs.gitea.com/usage/actions/secrets/)

The packaging script imports the certificate into a temporary keychain with a generated password. It restores the original keychain search list, deletes the temporary keychain, and removes temporary credential files on exit. The preceding build step receives no signing secrets and uses Tauri's `--no-sign` option; packaging then signs the helper and app explicitly. Do not enable shell tracing (`set -x`) around credential handling. Protect `main` and release tags so unreviewed changes cannot run with signing credentials.

## 5. Run and download

For the first run, open **Actions → macOS release → Run workflow**, choose **main**, and start it. Manual runs from other branches are skipped. Subsequent releases can run from a version tag such as `v0.1.0`; the tag must exactly match `v` plus the version in `src-tauri/tauri.conf.json`. Pull requests do not trigger this signing workflow. Protect the `v*` tag pattern in Gitea as well as `main`, because pushing a tag runs code with the signing secrets.

The job verifies the app's code signature, notarization ticket, and Gatekeeper assessment, then does the corresponding checks for the DMG. Each Apple submission waits for up to 90 minutes; Apple's first notarization submission may take longer than later submissions. Failure leaves the job failed; an unsigned or rejected package is not presented as a successful release. [Apple notarization overview](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution/)

Download **`Solar-macos-arm64`** from the successful run's **Artifacts** section. It contains the DMG, `SHA256SUMS`, and available notarization logs. The workflow uses Gitea's v4-compatible upload action pinned to a commit; build and artifact files live in job-specific paths under the runner's temporary directory and are removed after upload. Artifacts are retained for 14 days. Test the downloaded DMG on a Mac: mount it, drag Solar to Applications, launch it, and exercise screen sharing and Accessibility permissions. Signing/notarization checks do not replace that functional test. [Gitea artifacts](https://docs.gitea.com/usage/actions/artifacts/)

This initial workflow builds Apple Silicon only. Solar's helper staging currently supports separate `aarch64-apple-darwin` and `x86_64-apple-darwin` builds, not a universal helper; an Intel or universal release needs a separate extension to the pipeline.
