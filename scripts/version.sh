#!/usr/bin/env bash
# Computes the SemVer version of a build from the GitFlow branch and the release tags.
#
#   release/X.Y.Z, hotfix/X.Y.Z   -> X.Y.Z-rc.N    (N = commits since the branch left main)
#   main or detached HEAD on vX.Y.Z -> X.Y.Z       (official release)
#   anything else                  -> 0.0.0+<sha>  (unofficial build, e.g. develop, feature/*)
#
# Usage:
#   version.sh [--github]                  print the version of HEAD; --github also writes
#                                          version/core/official/prerelease to $GITHUB_OUTPUT
#   version.sh next major|minor|patch      print the version after the latest vX.Y.Z tag
#
# The branch is taken from $FREQFLIP_BRANCH, $GITHUB_HEAD_REF (pull requests), $GITHUB_REF_NAME
# (pushes) or the checked out branch, in that order. Needs the full history and tags
# (actions/checkout with fetch-depth: 0).
set -euo pipefail

readonly SEMVER_CORE='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

die() {
    echo "version.sh: $*" >&2
    exit 1
}

# Highest vX.Y.Z tag, optionally only among the tags on a commit ($1).
latest_tag() {
    local tags
    if [[ $# -gt 0 ]]; then
        tags=$(git tag --points-at "$1")
    else
        tags=$(git tag --list 'v*')
    fi
    grep -E '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' <<<"$tags" | sort -V | tail -n 1 || true
}

current_branch() {
    if [[ -n "${FREQFLIP_BRANCH:-}" ]]; then
        echo "$FREQFLIP_BRANCH"
    elif [[ -n "${GITHUB_HEAD_REF:-}" ]]; then
        echo "$GITHUB_HEAD_REF"
    elif [[ "${GITHUB_REF_TYPE:-}" == "branch" && -n "${GITHUB_REF_NAME:-}" ]]; then
        echo "$GITHUB_REF_NAME"
    else
        git symbolic-ref --short -q HEAD || true
    fi
}

# Commits on HEAD since it forked from main (origin/main in CI clones).
commits_since_main() {
    local main
    for main in origin/main main; do
        if git rev-parse -q --verify "$main^{commit}" >/dev/null; then
            git rev-list --count "$(git merge-base HEAD "$main")..HEAD"
            return
        fi
    done
    git rev-list --count HEAD
}

current_version() {
    local branch tag
    branch=$(current_branch)
    case "$branch" in
        release/* | hotfix/*)
            local core=${branch#*/}
            [[ "$core" =~ $SEMVER_CORE ]] || die "branch '$branch' must be named ${branch%%/*}/X.Y.Z"
            echo "$core-rc.$(commits_since_main)"
            return
            ;;
        main | "")
            tag=$(latest_tag HEAD)
            if [[ -n "$tag" ]]; then
                echo "${tag#v}"
                return
            fi
            ;;
    esac
    echo "0.0.0+$(git rev-parse --short=7 HEAD)"
}

next_version() {
    local tag major minor patch
    tag=$(latest_tag)
    tag=${tag:-v0.0.0}
    IFS=. read -r major minor patch <<<"${tag#v}"
    case "${1:-}" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "$major.$((minor + 1)).0" ;;
        patch) echo "$major.$minor.$((patch + 1))" ;;
        *) die "usage: version.sh next major|minor|patch" ;;
    esac
}

github_outputs() {
    local version=$1 core official=false prerelease=false
    core=${version%%[-+]*}
    [[ "$version" == "$core" ]] && official=true
    [[ "$version" == *-* ]] && prerelease=true
    {
        echo "version=$version"
        echo "core=$core"
        echo "official=$official"
        echo "prerelease=$prerelease"
    } >>"${GITHUB_OUTPUT:?GITHUB_OUTPUT is not set}"
}

case "${1:-}" in
    next)
        next_version "${2:-}"
        ;;
    --github)
        version=$(current_version)
        github_outputs "$version"
        echo "$version"
        ;;
    "")
        current_version
        ;;
    *)
        die "unknown argument '$1'"
        ;;
esac
