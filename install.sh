#!/bin/sh
# Installs tock, the terminal countdown timer and stopwatch:
#
#   curl -fsSL https://raw.githubusercontent.com/marcusbandit/ascii-countdown/main/install.sh | sh
#
# Puts the binary in ~/.local/bin and tab completion for zsh and bash under
# ~/.local/share. Settings, all optional:
#
#   TOCK_BIN_DIR    where the binary goes (default ~/.local/bin)
#   TOCK_VERSION    a release such as v0.1.0 or 0.1.0 (default: the latest)
#
# Everything runs from main() on the last line, so a download cut short
# runs nothing at all.
set -eu

say() { printf 'tock: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d' ' -f1
  elif command -v openssl >/dev/null 2>&1; then
    openssl dgst -sha256 -r "$1" | cut -d' ' -f1
  else
    die "needs sha256sum, shasum or openssl to check the download"
  fi
}

main() {
  repo=marcusbandit/ascii-countdown
  bin_dir=${TOCK_BIN_DIR:-$HOME/.local/bin}
  data_dir=${XDG_DATA_HOME:-$HOME/.local/share}
  version=${TOCK_VERSION:-latest}
  case $version in
    latest | v*) ;;
    *) version=v$version ;;
  esac

  [ "$(uname -s)" = Linux ] || die "only Linux builds exist, this is $(uname -s)"
  case $(uname -m) in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "no build for $(uname -m), only x86_64 and aarch64" ;;
  esac

  if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
  elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
  else
    die "needs curl or wget"
  fi

  if [ "$version" = latest ]; then
    base=https://github.com/$repo/releases/latest/download
  else
    base=https://github.com/$repo/releases/download/$version
  fi
  name=tock-$arch-linux.tar.gz

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  say "downloading $name ($version)"
  fetch "$base/$name" "$tmp/$name" || die "download failed: $base/$name"
  fetch "$base/$name.sha256" "$tmp/$name.sha256" || die "download failed: $base/$name.sha256"
  expected=$(cut -d' ' -f1 "$tmp/$name.sha256")
  [ "$(sha256_of "$tmp/$name")" = "$expected" ] || die "checksum mismatch, nothing installed"

  # Check the whole tarball before installing any of it.
  tar -xzf "$tmp/$name" -C "$tmp"
  for f in tock completions/_tock completions/tock.bash; do
    [ -f "$tmp/$f" ] || die "$name has no $f, nothing installed"
  done

  mkdir -p "$bin_dir" "$data_dir/zsh/site-functions" "$data_dir/bash-completion/completions"
  cp "$tmp/completions/_tock" "$data_dir/zsh/site-functions/_tock"
  cp "$tmp/completions/tock.bash" "$data_dir/bash-completion/completions/tock"
  # Copy then rename, so a running tock is never overwritten in place.
  cp "$tmp/tock" "$bin_dir/.tock.new"
  chmod 755 "$bin_dir/.tock.new"
  mv -f "$bin_dir/.tock.new" "$bin_dir/tock"

  say "installed $("$bin_dir/tock" --version) to $bin_dir/tock"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "$bin_dir is not on your PATH; add it in your shell's rc file" ;;
  esac
  say "try: tock 25m"
}

main "$@"
