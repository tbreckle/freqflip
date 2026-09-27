# GitHub Actions & CI/CD

GitHub Actions workflows for building, testing and releasing FreqFlip. The branching model is
GitFlow (see [GITFLOW.md](../GITFLOW.md)), versions are SemVer and computed by
[`scripts/version.sh`](../scripts/version.sh).

## Versioning

No version is committed: `Cargo.toml` stays at `0.0.0`. Every build gets its version
injected:

| Branch / ref | Version |
|---|---|
| tag `vX.Y.Z` (official release, built by Release Finish) | `X.Y.Z` |
| `release/X.Y.Z`, `hotfix/X.Y.Z` (and their PRs) | `X.Y.Z-rc.N` (N = commits since the branch left `main`) |
| `develop`, `feature/*`, `bugfix/*`, anything else | `0.0.0+<sha>` (unofficial) |

The `FREQFLIP_VERSION` env var goes through `build.rs` into `env!("FREQFLIP_VERSION")`
(`--version`, tray menu, Windows file properties).

## Workflows

### 1. CI (`ci.yml`)

**Triggers**: push to `develop`, `feature/**`, `bugfix/**`, `release/**`, `hotfix/**`; PRs to
`main`/`develop`; manual. No path filters, so required status checks always report.

**Jobs**: `Version` (runs `version.sh`), then `Build` (the reusable `build.yml`).

### 2. Build (`build.yml`, reusable)

Called by CI and Release Finish with a `version` (and optionally a `ref`).

| Job | What it does | Artifact |
|---|---|---|
| App (linux-x86_64) | test + build | `freqflip-<version>-linux-x86_64.tar.gz` |
| App (windows-x86_64) | test + build (with the icon resource) | `freqflip-<version>-windows-x86_64.zip` |
| App (macos-universal) | test + build for arm64 and x86_64, `lipo` | `freqflip-<version>-macos-universal.tar.gz` |
| Clippy (linux / windows / macos) | `cargo clippy -D warnings`, per platform because each has its own backend | |
| Format | `cargo fmt --check` | |
| Scripts | shellcheck + `scripts/test.sh` | |

### 3. Release Start (`release-start.yml`)

**Trigger**: manual (Actions → Release Start → Run workflow).

**Inputs**: `kind` = `release` (from `develop`) or `hotfix` (from `main`, always a patch bump);
`bump` = `major`/`minor`/`patch`.

**Steps**: next version from the latest `vX.Y.Z` tag → branch `release/X.Y.Z` or `hotfix/X.Y.Z`
→ CHANGELOG `[Unreleased]` becomes `[X.Y.Z] - date` (`scripts/changelog.sh release`) → PR to `main`.

### 4. Release Finish (`release-finish.yml`)

**Trigger**: a `release/*` or `hotfix/*` PR is merged into `main`.

**Steps**:
1. Version from the branch name, fails if the tag exists already
2. Build `X.Y.Z` from the merge commit (`build.yml`)
3. GitHub Release `vX.Y.Z` (creates the tag on the merge commit) with all artifacts, `SHA256SUMS`,
   and notes from the CHANGELOG section plus GitHub's generated PR list
4. Back-merge PR into `develop` from `backmerge/X.Y.Z` (the merge commit with `develop` merged
   in, so the PR is up to date). On a merge conflict, the branch is pushed without the merge and
   the PR describes how to resolve it.
5. Deletes the release/hotfix branch

Publishing steps are idempotent, so a failed `Publish` job can be re-run.

## Repository setup

1. **`main` branch**: must exist (created from `develop` for the first release). Without any
   `vX.Y.Z` tag the first release is `0.1.0` (minor) or `1.0.0` (major).
2. **Settings → Actions → General**: "Workflow permissions" read, and enable "Allow GitHub
   Actions to create and approve pull requests".
3. **Settings → General → Pull Requests**: allow merge commits. Release and back-merge PRs must be
   merged with **Create a merge commit**; squash/rebase breaks the GitFlow history.
4. **`RELEASE_TOKEN` secret (recommended)**: branches and PRs created with `GITHUB_TOKEN` don't
   trigger workflows, so CI would not run on the release and back-merge PRs, and their required
   checks would never report. Add a fine-grained PAT (or a GitHub App token) for this repository
   with *Contents* and *Pull requests* read/write. Without it, the workflows fall back to
   `GITHUB_TOKEN`; close and reopen the PR to start CI.
5. **Branch protection** on `main` and `develop`: 1 review, up to date, required checks
   `Build / App (linux-x86_64)`, `Build / App (windows-x86_64)`, `Build / App (macos-universal)`,
   `Build / Clippy (linux)`, `Build / Clippy (windows)`, `Build / Clippy (macos)`,
   `Build / Format`, `Build / Scripts`.

## Local use

```bash
scripts/version.sh                  # version of the current checkout
scripts/version.sh next minor       # next release version
scripts/test.sh                     # tests for version.sh and changelog.sh
scripts/changelog.sh notes 1.0.0    # release notes of a version

# Build with an injected version
FREQFLIP_VERSION=$(scripts/version.sh) cargo build --release
```

## Troubleshooting

- **Release PR has no checks**: `RELEASE_TOKEN` is missing, see Repository setup.
- **"GitHub Actions is not permitted to create or approve pull requests"**: `RELEASE_TOKEN` is
  missing and the Actions setting from Repository setup step 2 is off. The branch is pushed
  anyway: enable the setting (or add the token) and re-run the job, or open the PR from the link
  in the error. For a back-merge, Release Finish still deletes the release/hotfix branch.
- **Release Finish did not run**: the PR head must be `release/X.Y.Z` or `hotfix/X.Y.Z`, and the
  PR must be merged, not closed.
- **"Tag vX.Y.Z already exists"**: the version was released already; start a new release.
- **Back-merge conflicts**: resolve on `backmerge/X.Y.Z` as described in the PR. Usually
  CHANGELOG.md: keep new `develop` entries under `[Unreleased]`.
- **Windows build can't embed the icon**: `winresource` needs the Windows SDK's `rc.exe`
  (preinstalled on GitHub's Windows runners, part of the "Desktop development with C++" workload).
