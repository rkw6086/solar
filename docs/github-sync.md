# Gitea and GitHub synchronization

Gitea at <https://git.ryserve.net/internal/solar-chat> remains the primary development repository. The public mirror is <https://github.com/rkw6086/solar>. Push development branches and version tags only to the local `origin` remote; the Gitea **Sync GitHub** job handles replication. GitHub Actions owns all checks, builds, signing, and new release downloads.

The sync runs after `main` pushes, every five minutes, and on manual dispatch. It checks out only the trusted implementation from Gitea `main`, then reconciles refs in an isolated bare repository without executing incoming source or hooks.

- A branch present on one side is created on the other.
- Ahead branches fast-forward the other side.
- Clean divergence is merged with Gitea as the first parent and published to Gitea before GitHub.
- Conflicts, unrelated histories, and concurrent changes are reported and preserved for review. No branch is force-pushed.
- Branch deletion is manual: a branch surviving on either side will be restored on the other.
- Numbered version tags flow only from Gitea to GitHub. Existing tags are never overwritten.
- Rolling `beta`/`nightly` and temporary staging tags stay local to each release service.
- Reachable Git LFS objects transfer before refs; identical refs do not download repository objects or LFS payloads.

Git synchronization does not copy issues, pull requests, review state, secrets, workflow runs, or release records. Fork pull requests remain proposals until merged. Repository writers on either side can change the primary source through sync.

## Configuration

Gitea repository Actions settings contain:

| Setting | Purpose |
| --- | --- |
| Secret `SOLAR_GITHUB_SYNC_KEY` | Private half of a write-enabled deploy key registered only on `rkw6086/solar` |
| Variable `SOLAR_GITHUB_SYNC_ENABLED=true` | Explicitly enables public synchronization after publication checks |

The short-lived Gitea job token grants repository `code: write`. No personal API token or SSH identity is installed in the job. The deploy key is written to an owner-only temporary file and removed on exit; GitHub host keys are pinned in `scripts/ci/github-known-hosts`. The credential used to register a GitHub deploy key must have workflow permission to replicate workflow files.

Read-only inspection with these credentials configured:

```sh
python3 scripts/ci/sync-repositories.py --peer github --dry-run
```

For initial setup, disable GitHub Actions while seeding historical refs to avoid building every old version tag. Configure signing secrets, then enable Actions and publish a new stable version and a beta from the cleaned source. Never copy old binaries containing embedded credentials to a public release. Before enabling public sync, remove credentials from both the current tree and every history being published; a clean current tree alone does not clean its history.

The November 2026 public migration included a one-time rewrite removing an embedded logging credential from historical source. Historical commit and tag object IDs changed; their names and version numbers remain. Use a fresh clone for work based on the old history, rather than merging it back. The synchronization job rejects the original credential-bearing blob so an old checkout cannot republish it. Numbered tags created after this migration are immutable as usual.
