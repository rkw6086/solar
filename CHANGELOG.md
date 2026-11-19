# Changelog

Solar uses stable Semantic Versioning tags without a prefix, such as `0.1.0`.
Released tags and their downloads are immutable; changes receive a new version.

## [Unreleased]

### Added

- Editable random Orbit names and personalities, distinctive faces, and a gender-free setup form.
- Solar System, multi-Orbit, and multi-project memories with editable sharing.
- Orbit-created routines with titles, Markdown details, and a visual cron schedule editor.
- Animated resonant desktop orbits, larger teams, idle sun antics, draggable transforming chats, and 30-second idle collapse.
- Explicitly approved home-file imports with private copies and static profile dependency discovery.
- Guided CLI sign-in setup in Orbit settings, with a terminal using that Orbit's private configuration.

### Changed

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
