#!/usr/bin/env bash
# CHANGELOG.md helpers for the release workflows (Keep a Changelog format).
#
# Usage:
#   changelog.sh release X.Y.Z [DATE] [FILE]   start a "## [X.Y.Z] - DATE" section with the
#                                              entries of "## [Unreleased]", which stays empty
#   changelog.sh notes X.Y.Z [FILE]            print the entries of X.Y.Z (empty categories
#                                              are dropped), e.g. as GitHub Release notes
set -euo pipefail

die() {
    echo "changelog.sh: $*" >&2
    exit 1
}

release() {
    local version=$1 date=$2 file=$3 tmp
    [[ -f "$file" ]] || die "$file not found"
    if grep -qF "## [$version]" "$file"; then
        die "$file already has a section for $version"
    fi
    tmp=$(mktemp)
    awk -v heading="## [$version] - $date" '
        !done && $0 == "## [Unreleased]" { print; print ""; print heading; done = 1; next }
        !done && /^## \[/ { print "## [Unreleased]"; print ""; print heading; print ""; done = 1 }
        { print }
        END { if (!done) exit 1 }
    ' "$file" >"$tmp" || {
        rm -f "$tmp"
        die "no release section found in $file"
    }
    mv "$tmp" "$file"
}

notes() {
    local version=$1 file=$2
    [[ -f "$file" ]] || die "$file not found"
    awk -v heading="## [$version]" '
        # Prints a "### Category" heading only once an entry follows it.
        function flush_blank() { if (blank && printed) print ""; blank = 0 }
        index($0, heading) == 1 { inside = 1; next }
        inside && (/^## / || /^---/) { exit }
        !inside { next }
        /^### / { category = $0; blank = 0; next }
        /^[[:space:]]*$/ { blank = 1; next }
        {
            if (category != "") { if (printed) print ""; print category; print ""; category = ""; blank = 0 }
            flush_blank()
            print
            printed = 1
        }
    ' "$file"
}

case "${1:-}" in
    release)
        [[ -n "${2:-}" ]] || die "usage: changelog.sh release X.Y.Z [DATE] [FILE]"
        release "$2" "${3:-$(date -u +%Y-%m-%d)}" "${4:-CHANGELOG.md}"
        ;;
    notes)
        [[ -n "${2:-}" ]] || die "usage: changelog.sh notes X.Y.Z [FILE]"
        notes "$2" "${3:-CHANGELOG.md}"
        ;;
    *)
        die "usage: changelog.sh release|notes X.Y.Z ..."
        ;;
esac
