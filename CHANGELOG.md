# Changelog

Solar uses stable Semantic Versioning tags without a prefix, such as `0.1.0`.
Numbered Git tags are immutable. Downloads retain one stable release and one rolling beta; verified replacements retire older artifacts.

## [Unreleased]

### Added

- Four-corner desktop docking: the collapsed sun sits flush against the usable screen edge, moves inward on expansion, and returns to its saved corner when gathered.
- Manual clean macOS build checks with pinned Rust tooling and visible native compiler diagnostics.
- More Sprite eyes, expressions, planet surfaces, and rings, with distinct regenerated appearances.
- A larger visiting rocket, a passing comet, and a friendly flying saucer around the idle sun, with quiet, occasional animations and reduced-motion support.
- Read-only-by-default shared folder links for selected or all current and future Sprites, plus opt-in setup discovery of home configuration and global agent folders.
- Metadata-only home search and always-approved requests to add live links to a Sprite workspace.

### Fixed

- Split launcher and widget bundles so production frontend builds stay below the chunk warning limit; replace deprecated CI artifact downloads.
- Show a hand pointer over clickable desktop companions even while Solar is unfocused.
- Workspace-anchored file access and process filesystem allowlists restrict unshared host data; permission changes cancel work and invalidate cached sessions.
- Expanded desktop Sprites remain visible until the sun is clicked; idle time and clicks elsewhere no longer gather them. Clicking away from a chat still returns it to its planet.
- Remove text labels beneath desktop companions and the separate arrow launcher button; right-clicking the sun still opens Solar.
- Read widget hover in native window coordinates across mixed-density Mac displays, keep companion windows from taking keyboard focus, and toggle expansion against native state on the first click.

## [0.1.6]

### Fixed

- Recheck GitHub's temporary artifact deletion before declaring cleanup complete, allowing its API to catch up after a successful delete while still failing if artifacts remain.

## [0.1.5]

### Fixed

- Use Apple's supported argument order when verifying the architecture of the app and native helper in GitHub macOS builds.

### Changed

- Move all builds, checks, signing, and release publication to GitHub Actions, with Gitea remaining the primary source and securely synchronizing with the public `rkw6086/solar` repository.
- Remote diagnostics require a runtime `RYSERVE_LOGGING_KEY`; distributed source and new binaries contain no embedded logging credential.
- Keep one stable release and one rolling beta with verified replacement and cleanup of superseded downloads. Nightly beta checks run at 3:00 a.m. America/Chicago; About Solar identifies beta builds by source commit.
- Give every Sprite a separate orbital path in the launcher and desktop formation, with synchronized motion that keeps neighbors apart.
- Smooth desktop expansion, contraction, and orbiting; hovering or focusing a Sprite pauses the formation and gently enlarges its avatar. Remove the gold hover decorations.
- Regenerate a Sprite’s avatar from its settings or profile without changing its identity, instructions, model, or saved work.

## [0.1.3]

### Changed

- Renamed agent companions from Orbits to Sprites throughout the interface, onboarding, notifications, and agent instructions. Existing profiles, storage paths, and integrations stay compatible.

## [0.1.2]

**Upgrading:** Reconnect existing providers and re-enter saved MCP secrets once. Credentials now use owner-only, unencrypted files under `~/.solar/credentials`. Existing browser sessions may also need a fresh sign-in.

### Added

- Slowly moving Solar flares, independent of the star and face, with reduced-motion support.
- Editable random Orbit names and personalities, distinctive faces, and a gender-free setup form.
- Solar System, multi-Orbit, and multi-project memories with editable sharing.
- Orbit-created routines with titles, Markdown details, and a visual cron schedule editor.
- Animated resonant desktop orbits, larger teams, idle sun antics, draggable transforming chats, and 30-second idle collapse.
- Explicitly approved home-file imports with private copies and static profile dependency discovery.
- Guided CLI sign-in setup in Orbit settings, with a terminal using that Orbit's private configuration.

### Changed

- Provider, subscription, and MCP credentials use private files under `~/.solar/credentials` instead of macOS Keychain. Existing connections need a fresh sign-in.
- Solar browser and CLI defaults avoid Keychain prompts, with private browser profiles and file-based command-line credentials.
- Removed the standalone Activity screen; tool details and approvals stay in conversations.

### Fixed

- Chat opens at the latest message and follows streamed text and tool output until the user scrolls up.
- Shell commands use the account's default shell and the Orbit's own startup files while preserving separate credentials and project access rules.
- Remote shell diagnostics report command exit codes and failures instead of treating every completed process as successful.

## [0.1.1]

### Added

- Five Orbit tools for listing, creating, updating, deleting, and running routines, with readable activity cards and existing approval controls.

### Fixed

- Revalidate routine project access before scheduling and keep stale routines from blocking other work.
- Preserve scheduling state during edits and prevent deleted routines from being restored by a scheduler update.

## [0.1.0]

### Added

- Initial Solar desktop release for Apple Silicon Macs.
- User-created Orbits with individual and group conversations, configurable personalities, and model selection.
- Local project tools, shell execution, MCP integrations, and live tool activity.
- Desktop companions, attention notifications, and guided setup.
- Signed and notarized DMG distribution through Gitea Releases.
