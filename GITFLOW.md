# FreqFlip Development Workflow

This project uses **GitFlow** branching model for release management and continuous integration with GitHub Actions.

## Branch Structure

### Main Branches

- **`main`** - Production-ready code
  - Protected branch (requires pull request reviews)
  - Automatically tagged with version releases
  - All code must pass CI/CD checks

- **`develop`** - Development integration branch
  - Protected branch (requires pull request reviews)
  - Base branch for feature development
  - Should always be in a working state

### Supporting Branches

- **`feature/*`** - Feature branches (e.g., `feature/kms-backend`)
  - Created from: `develop`
  - Merged back into: `develop`
  - Naming convention: `feature/descriptive-name`

- **`bugfix/*`** - Bug fix branches (e.g., `bugfix/restore-retry`)
  - Created from: `develop`
  - Merged back into: `develop`
  - Naming convention: `bugfix/issue-number-description`

- **`release/*`** - Release branches (e.g., `release/1.0.0`)
  - Created from: `develop`
  - Merged into: `main` and back to `develop`
  - Automatically created by release workflow
  - Only version bumps and critical fixes

- **`hotfix/*`** - Hotfix branches (e.g., `hotfix/1.0.1`)
  - Created from: `main`
  - Merged into: `main` and `develop`
  - For critical production fixes

## Workflow

### Creating a Feature

```bash
# Create feature branch from develop
git checkout develop
git pull origin develop
git checkout -b feature/my-feature

# Work on feature
# ... make commits ...

# Push and create pull request
git push origin feature/my-feature
```

Then create a Pull Request on GitHub targeting the `develop` branch.

### Creating a Release

1. Go to GitHub Actions → **Release Start** → "Run workflow"
2. Choose `kind: release` and the part to bump (`major`/`minor`/`patch`)
3. The workflow:
   - computes the next version from the latest `vX.Y.Z` tag (`scripts/version.sh next <bump>`)
   - creates `release/X.Y.Z` from `develop`
   - moves the `[Unreleased]` CHANGELOG entries into a `[X.Y.Z]` section
   - opens a pull request to `main`
4. CI builds the release branch as `X.Y.Z-rc.N`. Only fixes for this release go onto it.
5. Merge the PR with **Create a merge commit** (no squash/rebase). **Release Finish** then:
   - builds `X.Y.Z` from the merge commit
   - creates the tag `vX.Y.Z` and the GitHub Release (Linux/Windows/macOS archives, `SHA256SUMS`, notes from CHANGELOG.md)
   - opens a back-merge PR from `main` into `develop` and deletes the release branch
6. Review and merge the back-merge PR (again with a merge commit). Check that CHANGELOG entries
   added to `develop` in the meantime stay under `[Unreleased]`.

No version numbers are committed: `Cargo.toml` stays at `0.0.0`,
CI injects the version at build time. So the back-merge brings no version into `develop`.

### Hotfix (Critical Production Fixes)

1. GitHub Actions → **Release Start** with `kind: hotfix` creates `hotfix/X.Y.Z` (patch bump) from `main` and opens a PR to `main`
2. Push the fix to the hotfix branch and add it to the `[X.Y.Z]` section in CHANGELOG.md
3. Merge the PR: the same **Release Finish** steps as for a release run, including the back-merge PR into `develop`

## Versioning

Versions follow [Semantic Versioning](https://semver.org/) and are computed from the branch and
the tags by `scripts/version.sh` (a small replacement for GitVersion):

| Branch / ref | Version | Meaning |
|---|---|---|
| tag `vX.Y.Z` (on `main`) | `X.Y.Z` | official release |
| `release/X.Y.Z`, `hotfix/X.Y.Z` | `X.Y.Z-rc.N` | release candidate, N = commits since the branch left `main` |
| `develop`, `feature/*`, `bugfix/*`, anything else | `0.0.0+<sha>` | unofficial build |

- **MAJOR** (X.0.0) - Breaking changes (e.g. changed command line arguments or exit codes that break launch scripts)
- **MINOR** (0.X.0) - New features, backward compatible
- **PATCH** (0.0.X) - Bug fixes

The version is injected into the app at build time (`FREQFLIP_VERSION`, shown by
`freqflip --version` and in the tray menu).

```bash
scripts/version.sh              # version of the current checkout
scripts/version.sh next minor   # next minor release version
FREQFLIP_VERSION=$(scripts/version.sh) cargo build --release   # local build with version
```

## CI/CD Pipelines

See [.github/WORKFLOWS.md](.github/WORKFLOWS.md).

- **CI** (`ci.yml`): every push to a GitFlow branch and every PR to `main`/`develop`: version, build + tests (Linux, Windows, macOS universal), clippy on all three, fmt, script tests
- **Release Start** (`release-start.yml`): manual, creates the release/hotfix branch and PR
- **Release Finish** (`release-finish.yml`): on merge of a release/hotfix PR into `main`, tags, builds and publishes

## Branch Protection Rules

Configured on `main` and `develop`:

- ✅ Require pull request reviews (minimum 1)
- ✅ Require status checks to pass (all CI/CD workflows)
- ✅ Require branches to be up to date before merging
- ✅ Require code review dismissal when new commits pushed
- ✅ Enforce all configured restrictions for administrators

## Common Commands

```bash
# Ensure local branches are up to date
git fetch origin

# Create and push a feature branch
git checkout -b feature/my-feature origin/develop
git push -u origin feature/my-feature

# Update develop from origin
git checkout develop
git pull origin develop

# Merge another branch locally (avoid if possible, use PRs)
git merge origin/some-branch
```

## Tips

- Keep feature branches short-lived (few days maximum)
- Write clear commit messages
- Link PRs to related issues
- Use meaningful branch names
- Test locally before pushing
- Ensure CI passes before requesting review
