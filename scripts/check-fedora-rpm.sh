#!/usr/bin/env bash
# Run only inside the disposable Fedora CI container. No host installation.
set -euo pipefail
package=$(realpath "$1")
version=$2
dnf install -y "$package" desktop-file-utils
[[ "$(rpm -q --queryformat '%{VERSION}' marktub)" == "$version" ]]
[[ "$(rpm -q --queryformat '%{ARCH}' marktub)" == x86_64 ]]
rpm --verify marktub
desktop-file-validate /usr/share/applications/Marktub.desktop
linked=$(ldd /usr/bin/marktub)
printf '%s\n' "$linked"
if [[ "$linked" == *'not found'* ]]; then exit 1; fi
printf '%s\n' 'PASS: Fedora RPM installation, package ownership, version, desktop entry and runtime library resolution'
