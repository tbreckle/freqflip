#!/usr/bin/env bash
# Tests version.sh and changelog.sh against a throwaway git repository.
set -euo pipefail

scripts=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

check() {
    local name=$1 expected=$2 actual=$3
    if [[ "$actual" == "$expected" ]]; then
        echo "ok   $name"
    else
        echo "FAIL $name: expected '$expected', got '$actual'"
        failures=$((failures + 1))
    fi
}

# Run version.sh as on a developer machine, without GitHub Actions variables.
version() {
    env -u GITHUB_HEAD_REF -u GITHUB_REF_NAME -u GITHUB_REF_TYPE -u FREQFLIP_BRANCH "$scripts/version.sh" "$@"
}

cd "$work"
git init -q -b main
git config user.name test
git config user.email test@example.com
git config commit.gpgsign false
git config tag.gpgsign false
commit() { git commit -q --allow-empty -m "$1"; }

commit "initial"
check "no tags, next minor" "0.1.0" "$(version next minor)"
check "untagged main is unofficial" "0.0.0+$(git rev-parse --short=7 HEAD)" "$(version)"

git tag v1.0.0
check "tagged main" "1.0.0" "$(version)"
git tag v1.0.0-rc.1
git tag vfoo
check "ignores non-release tags" "1.1.0" "$(version next minor)"
check "next major" "2.0.0" "$(version next major)"
check "next patch" "1.0.1" "$(version next patch)"

git switch -q -c develop
check "develop on a tagged commit stays unofficial" "0.0.0+$(git rev-parse --short=7 HEAD)" "$(version)"
commit "feature"
commit "feature 2"
check "develop" "0.0.0+$(git rev-parse --short=7 HEAD)" "$(version)"

git switch -q -c release/1.1.0
commit "changelog"
check "release branch" "1.1.0-rc.3" "$(version)"
check "FREQFLIP_BRANCH override" "0.0.0+$(git rev-parse --short=7 HEAD)" "$(FREQFLIP_BRANCH=develop "$scripts/version.sh")"
check "pull request head ref" "1.1.0-rc.3" "$(GITHUB_HEAD_REF=release/1.1.0 GITHUB_REF_NAME=5/merge GITHUB_REF_TYPE=branch "$scripts/version.sh")"

git switch -q main
git merge -q --no-ff -m "Merge release/1.1.0" release/1.1.0
check "merged but untagged main" "0.0.0+$(git rev-parse --short=7 HEAD)" "$(version)"
git tag v1.1.0
check "released main" "1.1.0" "$(version)"
git switch -q --detach HEAD
check "detached tagged HEAD" "1.1.0" "$(version)"
git switch -q main

git switch -q -c hotfix/1.1.1
commit "fix"
check "hotfix branch" "1.1.1-rc.1" "$(version)"

git switch -q -c release/1.2
check "invalid release branch fails" "1" "$(version >/dev/null 2>&1 && echo 0 || echo 1)"

GITHUB_OUTPUT="$work/out"
export GITHUB_OUTPUT
FREQFLIP_BRANCH=release/2.0.0 "$scripts/version.sh" --github >/dev/null
check "github outputs" "version=2.0.0-rc.1 core=2.0.0 official=false prerelease=true" "$(paste -sd ' ' "$work/out")"
unset GITHUB_OUTPUT

cat >CHANGELOG.md <<'EOF'
# Changelog

## [Unreleased]

### Added
- New thing

### Fixed

## [1.0.0] - 2026-02-01

### Added
- Initial release

---

## Guide
EOF
"$scripts/changelog.sh" release 1.1.0 2026-09-26
check "changelog release section" "## [Unreleased]||## [1.1.0] - 2026-09-26||### Added" \
    "$(sed -n '3,7p' CHANGELOG.md | paste -sd '|')"
check "changelog notes" "### Added||- New thing" "$("$scripts/changelog.sh" notes 1.1.0 | paste -sd '|')"
check "changelog notes stop at ---" "### Added||- Initial release" "$("$scripts/changelog.sh" notes 1.0.0 | paste -sd '|')"
check "changelog rejects duplicate" "1" "$("$scripts/changelog.sh" release 1.1.0 >/dev/null 2>&1 && echo 0 || echo 1)"

if ((failures > 0)); then
    echo "$failures test(s) failed"
    exit 1
fi
echo "All tests passed"
