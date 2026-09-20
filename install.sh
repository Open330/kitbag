#!/bin/sh
# The only shell that survives.
#
# Everything kitbag does is in a binary, and this fetches it. That is the one
# job shell is still the right tool for: a machine with nothing installed on it
# has `sh` and `curl`, and nothing else can be assumed.
#
#   curl -LsSf https://raw.githubusercontent.com/Open330/kitbag/main/install.sh | sh
#
# It detects the platform, downloads the matching release, checks it against
# the checksums published with it, and puts the binary in ~/.local/bin. It
# never writes anywhere else and never asks for a password.

set -eu

REPO="Open330/kitbag"
BIN="kitbag"
DEST="${KITBAG_BIN_DIR:-$HOME/.local/bin}"
VERSION="${KITBAG_VERSION:-latest}"

say() { printf '  %s\n' "$*"; }
die() { printf '  %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "this needs $1, and it is not here"
}

need curl
need tar

case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux)  os="unknown-linux-gnu" ;;
    *)      die "no build for $(uname -s) — build from source with cargo instead" ;;
esac

case "$(uname -m)" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64)  arch="x86_64" ;;
    *)             die "no build for $(uname -m) — build from source with cargo instead" ;;
esac

target="${arch}-${os}"

if [ "$VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
else
    base="https://github.com/$REPO/releases/download/$VERSION"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

say "fetching $BIN for $target"
curl -LsSf --proto '=https' --tlsv1.2 "$base/$BIN-$target.tar.gz" -o "$tmp/$BIN.tar.gz" \
    || die "no release for $target — build from source with cargo instead"
curl -LsSf --proto '=https' --tlsv1.2 "$base/checksums.txt" -o "$tmp/checksums.txt" \
    || die "the release has no checksums; refusing to install it"

# A download nobody checked is a download somebody else can choose.
want="$(grep " $BIN-$target.tar.gz\$" "$tmp/checksums.txt" | awk '{print $1}')"
[ -n "$want" ] || die "no checksum published for $target"

if command -v shasum >/dev/null 2>&1; then
    got="$(shasum -a 256 "$tmp/$BIN.tar.gz" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
    got="$(sha256sum "$tmp/$BIN.tar.gz" | awk '{print $1}')"
else
    die "neither shasum nor sha256sum is here, so the download cannot be checked"
fi

[ "$want" = "$got" ] || die "checksum mismatch — not installing this"

tar -xzf "$tmp/$BIN.tar.gz" -C "$tmp"
mkdir -p "$DEST"
mv "$tmp/$BIN" "$DEST/$BIN"
chmod 755 "$DEST/$BIN"

say "installed $DEST/$BIN"
case ":$PATH:" in
    *":$DEST:"*) ;;
    *) say "add it to your PATH:  export PATH=\"$DEST:\$PATH\"" ;;
esac
say "next:  $BIN discover"
