#!/bin/sh
# Installs tock, the terminal countdown timer and stopwatch:
#
#   curl -fsSL https://raw.githubusercontent.com/marcusbandit/ascii-countdown/main/install.sh | sh
#
# Puts the binary in ~/.local/bin and tab completion for zsh and bash under
# ~/.local/share. Settings, all optional:
#
#   TOCK_BIN_DIR    where the binary goes (default ~/.local/bin)
#   TOCK_VERSION    a release tag such as v0.1.0 (default: the latest release)
set -eu

repo=marcusbandit/ascii-countdown
bin_dir=${TOCK_BIN_DIR:-$HOME/.local/bin}
data_dir=${XDG_DATA_HOME:-$HOME/.local/share}
version=${TOCK_VERSION:-latest}

say() { printf 'tock: %s\n' "$*"; }
die() { say "$*" >&2; exit 1; }

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
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $name ($version)"
fetch "$base/$name" "$tmp/$name" || die "download failed: $base/$name"
fetch "$base/$name.sha256" "$tmp/$name.sha256" || die "download failed: $base/$name.sha256"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$tmp" && sha256sum -c "$name.sha256" >/dev/null) || die "checksum mismatch, nothing installed"
else
  say "sha256sum not found, skipping the checksum"
fi

tar -xzf "$tmp/$name" -C "$tmp"
mkdir -p "$bin_dir" "$data_dir/zsh/site-functions" "$data_dir/bash-completion/completions"
# Copy then rename, so a running tock is never overwritten in place.
cp "$tmp/tock" "$bin_dir/.tock.new"
chmod 755 "$bin_dir/.tock.new"
mv -f "$bin_dir/.tock.new" "$bin_dir/tock"
cp "$tmp/completions/_tock" "$data_dir/zsh/site-functions/_tock"
cp "$tmp/completions/tock.bash" "$data_dir/bash-completion/completions/tock"

say "installed $("$bin_dir/tock" --version) to $bin_dir/tock"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) say "$bin_dir is not on your PATH; add it in your shell's rc file" ;;
esac
case ${SHELL:-} in
  */zsh) say "zsh completion needs $data_dir/zsh/site-functions on \$fpath before compinit" ;;
esac
say "try: tock 25m"
