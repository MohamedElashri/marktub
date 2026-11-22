#!/usr/bin/env bash
set -euo pipefail

fail() { printf 'Marktub: %s\n' "$*" >&2; exit 1; }

fetch() {
  if [[ "$download_tool" == curl ]]; then
    curl --fail --silent --show-error --location --retry 2 --connect-timeout 15 \
      --max-time 180 --proto '=https' --proto-redir '=https' --output "$2" "$1"
  else
    wget --https-only --timeout=30 --tries=3 --quiet --output-document="$2" "$1"
  fi
}

main() {
  download_only=false
  download_dir=''
  package_format=''
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --download-only) download_only=true; shift ;;
      --directory)
        [[ $# -ge 2 && -n "$2" ]] || fail '--directory needs a path.'
        download_dir=$2; shift 2 ;;
      --package-format)
        [[ $# -ge 2 ]] || fail '--package-format needs deb or rpm.'
        case "$2" in deb|rpm) package_format=$2 ;; *) fail '--package-format needs deb or rpm.' ;; esac
        shift 2 ;;
      --help|-h)
        printf '%s\n' 'Install the latest Marktub release for Linux x86_64 or macOS.' \
          '  --download-only   Verify and save the package without installing/opening it.' \
          '  --package-format deb|rpm   Override Linux package format detection.' \
          '  --directory DIR   Save downloads here (default: current directory for' \
          '                    --download-only, Downloads on macOS).'
        return ;;
      *) fail "Unknown option: $1" ;;
    esac
  done

  platform=$(uname -s)
  architecture=$(uname -m)
  case "$platform/$architecture" in
    Linux/x86_64|Linux/amd64)
      if [[ -z "$package_format" ]]; then
        if command -v apt-get >/dev/null; then package_format=deb
        elif command -v dnf >/dev/null || command -v zypper >/dev/null; then package_format=rpm
        else fail 'Choose --package-format deb or rpm with --download-only, or install using your system package manager.'; fi
      fi
      case "$package_format" in
        deb) asset_pattern='_amd64\.deb$'; package_manager=apt-get ;;
        rpm)
          asset_pattern='_x86_64\.rpm$'
          if command -v dnf >/dev/null; then package_manager=dnf
          else package_manager=zypper; fi ;;
      esac
      if ! "$download_only"; then
        command -v "$package_manager" >/dev/null || fail "Install the verified $package_format download with your system package manager, or use --download-only."
        if [[ "$EUID" -ne 0 ]]; then
          command -v sudo >/dev/null || fail 'sudo is required to install the Linux package.'
        fi
      fi ;;
    Darwin/arm64|Darwin/x86_64)
      [[ -z "$package_format" ]] || fail '--package-format is only used on Linux.'
      asset_pattern='_universal\.dmg$' ;;
    *) fail "No released package for $platform/$architecture. See https://github.com/MohamedElashri/marktub/releases/latest" ;;
  esac

  if command -v curl >/dev/null; then download_tool=curl
  elif command -v wget >/dev/null; then download_tool=wget
  else fail 'Install curl or wget first.'; fi
  if command -v sha256sum >/dev/null; then hash_tool=sha256sum
  elif command -v shasum >/dev/null; then hash_tool=shasum
  else fail 'A SHA-256 tool (sha256sum or shasum) is required.'; fi

  install_workdir=$(mktemp -d "${TMPDIR:-/tmp}/marktub-install.XXXXXX")
  trap 'rm -rf "$install_workdir"' EXIT
  fetch 'https://api.github.com/repos/MohamedElashri/marktub/releases/latest' "$install_workdir/release.json"
  # Only read download URL strings, not arbitrary JSON values or release prose.
  asset_url=$(grep -Eo '"browser_download_url"[[:space:]]*:[[:space:]]*"https://github.com/MohamedElashri/marktub/releases/download/[A-Za-z0-9._-]+/Marktub_[A-Za-z0-9._-]+"' \
    "$install_workdir/release.json" | cut -d '"' -f 4 | grep -E "$asset_pattern" || true)
  case "$asset_url" in
    ''|*$'\n'*) fail 'The latest release has no unique package for this platform.' ;;
  esac
  filename=${asset_url##*/}
  package="$install_workdir/$filename"
  printf 'Downloading %s\n' "$filename"
  fetch "$asset_url" "$package"
  fetch "${asset_url%/*}/SHA256SUMS" "$install_workdir/SHA256SUMS"
  expected=$(awk -v name="$filename" '$2 == name { print $1 }' "$install_workdir/SHA256SUMS")
  [[ "$expected" =~ ^[a-f0-9]{64}$ ]] || fail 'Missing or ambiguous package checksum.'
  if [[ "$hash_tool" == sha256sum ]]; then actual=$(sha256sum "$package")
  else actual=$(shasum -a 256 "$package"); fi
  [[ "${actual%% *}" == "$expected" ]] || fail 'Checksum mismatch; the package was not installed.'

  if "$download_only" || [[ "$platform" == Darwin || -n "$download_dir" ]]; then
    if [[ -z "$download_dir" ]]; then
      if "$download_only"; then download_dir=$PWD
      else download_dir="$HOME/Downloads"; fi
    fi
    mkdir -p "$download_dir"
    saved_dir=$(mktemp -d "$download_dir/Marktub.XXXXXX")
    cp "$package" "$saved_dir/$filename"
    package="$saved_dir/$filename"
    printf 'Verified download: %s\n' "$package"
  fi
  "$download_only" && return 0
  if [[ "$platform" == Darwin ]]; then
    printf '%s\n' 'Opening the universal disk image. Drag Marktub to Applications.'
    open "$package"
  else
    # Allow apt's unprivileged download helper to read the verified public package.
    chmod 755 "$install_workdir"
    chmod 644 "$package"
    case "$package_manager" in
      apt-get|dnf) install_args=(install -y "$package") ;;
      zypper) install_args=(install "$package") ;;
    esac
    if [[ "$EUID" -eq 0 ]]; then "$package_manager" "${install_args[@]}"
    else sudo "$package_manager" "${install_args[@]}"; fi
  fi
}

main "$@"
