#!/usr/bin/env bash
# A release: the APK (with every model) signed with the release key — kept
# outside the repository — tagged and published on GitHub with the notes.
#
#   android/release.sh 1.0.0 notes.md
#
# The key: RELEASE_KEYS (default ~/.local/share/rust-kb-release: release.jks,
# release.pass). The version code: 1.2.3 → 10203, so each release installs
# over the one before. The build runs as android/build.sh does (one cargo
# job unless CARGO_BUILD_JOBS says otherwise; PYTHON: one with numpy).
set -euo pipefail
V=${1:?the version, e.g. 1.0.0}
NOTES=${2:?the release notes file}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
KEYS=${RELEASE_KEYS:-$HOME/.local/share/rust-kb-release}
IFS=. read -r MAJOR MINOR PATCH <<< "$V"
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "commit the changes first" >&2
    exit 1
fi
export VERSION_NAME=$V VERSION_CODE=$((MAJOR * 10000 + MINOR * 100 + PATCH))
export KEYSTORE=$KEYS/release.jks KEYSTORE_PASS=file:$KEYS/release.pass KEY_ALIAS=rust-kb
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-1}
# Nothing goes out with a failing test.
cargo test --release -q -p kbcore -p kbime --lib
nix develop -c android/build.sh
# Nor with commas that came right before and don't now (tests/commas-ru.tsv).
cargo build --release -q -p kbime --example typetext
if ! python3 tools/comma_suite.py > /dev/null; then
    python3 tools/comma_suite.py | grep -E "NEW|^all" >&2
    exit 1
fi
APK=target/apk/rust-kb-$V.apk
cp target/apk/rust-kb.apk "$APK"
git tag -a "v$V" -m "Rust KB $V"
git push origin "v$V"
gh release create "v$V" "$APK" --title "Rust KB $V" --notes-file "$NOTES"
