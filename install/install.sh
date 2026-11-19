#!/bin/sh
# Biwa のインストーラ (macOS / Linux)。
#
#   curl -sSf https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.sh | sh
#   curl -sSf https://raw.githubusercontent.com/biwa-engine/biwa/main/install/install.sh | sh -s -- --toolchain nightly
#
# やること:
#   1. GitHub Releases から、このホスト向けの CLI (`biwa-cli-<name>-<host>`) を落とし、SHA256SUMS で確かめる
#   2. その CLI に `biwa toolchain install <name> --default` をさせる
#      (ツールチェインを `~/.biwa/toolchains/<name>/` に入れ、ランチャー `~/.biwa/bin/biwa` を置く)
#   3. `~/.biwa/bin` に PATH を通す (`~/.biwa/env` を書き、シェルの設定ファイルから読む)
#
# オプション:
#   --toolchain <SPEC>  stable / nightly / 0.1.0 / <commit>。省略すると最新の版 (まだ無ければ最新の nightly)
#   --no-modify-path    シェルの設定ファイルを書き換えない
# 環境変数:
#   BIWA_HOME           入れる場所 (既定 ~/.biwa)
#   BIWA_RELEASES_URL   release の一覧 (既定 https://api.github.com/repos/biwa-engine/biwa/releases。試験用)
#   GITHUB_TOKEN        あれば GitHub の API に使う (回数の制限を避ける)

set -eu

spec=""
modify_path=1
while [ $# -gt 0 ]; do
  case "$1" in
    --toolchain) spec="${2:?--toolchain needs a value}"; shift 2 ;;
    --toolchain=*) spec="${1#--toolchain=}"; shift ;;
    --no-modify-path) modify_path=0; shift ;;
    -h|--help) sed -n '2,24p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "error: unknown option: $1" >&2; exit 1 ;;
  esac
done

say() { printf 'biwa-install: %s\n' "$*" >&2; }
die() { printf 'biwa-install: error: %s\n' "$*" >&2; exit 1; }

BIWA_HOME="${BIWA_HOME:-$HOME/.biwa}"
RELEASES_URL="${BIWA_RELEASES_URL:-https://api.github.com/repos/biwa-engine/biwa/releases}"

case "$(uname -s)" in
  Linux)
    case "$(uname -m)" in
      x86_64|amd64) host=linux-x86_64 ;;
      *) die "Biwa is not released for Linux $(uname -m) yet" ;;
    esac ;;
  Darwin) host=macos-universal ;;
  *) die "this script is for macOS and Linux (on Windows, use install.ps1)" ;;
esac

if command -v curl > /dev/null 2>&1; then
  fetch() {
    if [ -n "${GITHUB_TOKEN:-}" ] && [ "${2:-}" = api ]; then
      curl -sSfL -H "Authorization: Bearer $GITHUB_TOKEN" "$1"
    else
      curl -sSfL "$1"
    fi
  }
elif command -v wget > /dev/null 2>&1; then
  fetch() { wget -qO- "$1"; }
else
  die "curl or wget is required"
fi

if command -v sha256sum > /dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum > /dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "sha256sum or shasum is required"
fi

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t biwa-install)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "fetching the list of releases"
fetch "$RELEASES_URL?per_page=100" api > "$tmp/releases.json" || die "failed to fetch $RELEASES_URL"

# このホスト向けの CLI の URL を、新しい release から順に並べる (GitHub は新しい順に返す)。
# JSON は 1 行にまとめてから、資産の URL だけを抜き出す (jq に頼らない)。
tr -d '\n' < "$tmp/releases.json" \
  | grep -o '"browser_download_url"[[:space:]]*:[[:space:]]*"[^"]*"' \
  | sed 's/.*"\([^"]*\)"$/\1/' \
  | grep "/biwa-cli-[^/]*-$host\$" > "$tmp/clis" || true

# release のタグ (URL の `/download/<tag>/` の部分) で選ぶ。
tag_of() { printf '%s\n' "$1" | sed 's#.*/download/\([^/]*\)/[^/]*$#\1#'; }
pick() { # $1: タグに合う grep の正規表現。最初 (= 最新) の 1 つ
  while IFS= read -r url; do
    if tag_of "$url" | grep -q "$1"; then printf '%s\n' "$url"; return 0; fi
  done < "$tmp/clis"
  return 1
}
stable_re='^v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$'
case "$spec" in
  "") url="$(pick "$stable_re" || pick '^nightly-' || true)" ;;
  stable) url="$(pick "$stable_re" || true)" ;;
  nightly) url="$(pick '^nightly-' || true)" ;;
  [0-9]*.[0-9]*.[0-9]*) url="$(pick "^v${spec}\$" || true)" ;;
  v[0-9]*.[0-9]*.[0-9]*) url="$(pick "^${spec}\$" || true)" ;;
  *) url="$(pick "^nightly-${spec}\$" || true)" ;;
esac
[ -n "$url" ] || die "no released Biwa toolchain for $host matches '${spec:-the newest release}'"

asset="${url##*/}"
name="${asset#biwa-cli-}"
name="${name%-$host}"
say "installing the toolchain '$name' ($(tag_of "$url"))"

fetch "$url" > "$tmp/biwa" || die "failed to download $url"
fetch "${url%/*}/SHA256SUMS" > "$tmp/SHA256SUMS" || die "failed to download the checksums"
want="$(grep "[[:space:]]\*\{0,1\}$asset\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
got="$(sha256 "$tmp/biwa")"
[ -n "$want" ] || die "SHA256SUMS has no entry for $asset"
[ "$want" = "$got" ] || die "the checksum of $asset does not match (expected $want, got $got)"
chmod +x "$tmp/biwa"

# 落とした CLI にツールチェインを入れさせる (圧縮ファイルの取得・確認・展開・ランチャーの配置・既定の設定)。
mkdir -p "$BIWA_HOME"
BIWA_HOME="$BIWA_HOME" "$tmp/biwa" toolchain install "$name" --default

# PATH。
cat > "$BIWA_HOME/env" <<EOF
# Biwa (install.sh が書いた)。\`~/.biwa/bin\` に PATH を通す。
case ":\${PATH}:" in
  *":$BIWA_HOME/bin:"*) ;;
  *) export PATH="$BIWA_HOME/bin:\$PATH" ;;
esac
EOF

line=". \"$BIWA_HOME/env\""
if [ "$modify_path" = 1 ]; then
  for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshenv"; do
    # 無いファイルは .profile と、そのシェルが入っているときだけ作る。
    case "$rc" in
      */.bashrc) [ -f "$rc" ] || continue ;;
      */.zshenv) [ -f "$rc" ] || command -v zsh > /dev/null 2>&1 || continue ;;
    esac
    if [ -f "$rc" ] && grep -qF "$line" "$rc"; then continue; fi
    printf '\n%s\n' "$line" >> "$rc"
    say "added $BIWA_HOME/bin to PATH in $rc"
  done
fi

say "done. Biwa is installed in $BIWA_HOME"
case ":$PATH:" in
  *":$BIWA_HOME/bin:"*) ;;
  *) say "to use it in this shell, run: $line" ;;
esac
