#!/usr/bin/env bash
# Install an official, checksum-verified rclone in a disposable test directory.
# Usage: scripts/install-test-rclone.sh 1.68.0|latest DESTINATION
set -euo pipefail
version=${1:?usage: install-test-rclone.sh VERSION DESTINATION}
destination=${2:?usage: install-test-rclone.sh VERSION DESTINATION}
if [ "$version" = latest ]; then
  version=$(curl --fail --silent --show-error https://downloads.rclone.org/version.txt)
  version=${version#rclone v}
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "invalid rclone version: $version" >&2; exit 1; }
case $(uname -m) in
  x86_64) arch=amd64 ;;
  aarch64) arch=arm64 ;;
  *) echo 'unsupported test host architecture' >&2; exit 1 ;;
esac
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
archive="rclone-v$version-linux-$arch.zip"
url="https://downloads.rclone.org/v$version"
curl --fail --silent --show-error -o "$work/$archive" "$url/$archive"
curl --fail --silent --show-error -o "$work/SHA256SUMS" "$url/SHA256SUMS"
# SHA256SUMS is a clear-signed document: select exactly this archive's entry.
awk -v file="$archive" '$2 == file { print; count++ } END { if (count != 1) exit 1 }' \
  "$work/SHA256SUMS" > "$work/checksum"
(cd "$work" && sha256sum --check --strict checksum)
mkdir -p "$destination"
python3 - "$work/$archive" "rclone-v$version-linux-$arch/rclone" "$destination/rclone" <<'PY'
import pathlib
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as archive:
    pathlib.Path(sys.argv[3]).write_bytes(archive.read(sys.argv[2]))
PY
chmod 755 "$destination/rclone"
actual=$("$destination/rclone" version)
[[ "${actual%%$'\n'*}" = "rclone v$version" ]] || { echo "unexpected rclone version: $actual" >&2; exit 1; }
printf '%s\n' "$version" > "$destination/version"
printf '%s\n' "$actual"
