# Changelog

Solar uses stable Semantic Versioning tags without a prefix, such as `0.1.0`.
Released tags and their downloads are immutable; changes receive a new version.

## [Unreleased]

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
