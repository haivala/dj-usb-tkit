#!/usr/bin/env bash
# Prepares the AUR package dj-usb-tkit-bin for a published GitHub release.
#
#   scripts/aur-release.sh v0.6.2            # render PKGBUILD + .SRCINFO, test-build
#   scripts/aur-release.sh v0.6.2 --install  # ...and install the built package
#   PKGREL=2 scripts/aur-release.sh v0.6.2   # packaging-only fix for the same version
#
# The AUR repository is cloned into packaging/aur/dj-usb-tkit-bin/ (ignored by
# git here). The script stages PKGBUILD and .SRCINFO there but never commits or
# pushes; it prints the commands for that at the end.
set -euo pipefail

PKGNAME=dj-usb-tkit-bin
AUR_URL="ssh://aur@aur.archlinux.org/${PKGNAME}.git"
REPO_URL="https://github.com/haivala/dj-usb-tkit"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
template="$root/packaging/aur/PKGBUILD.in"
aur_dir="$root/packaging/aur/$PKGNAME"

tag="${1:-}"
install=0
[[ "${2:-}" == "--install" ]] && install=1

if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "usage: $0 vX.Y.Z [--install]  (stable release tags only, no -rc)" >&2
  exit 1
fi
version="${tag#v}"
pkgrel="${PKGREL:-1}"

for tool in curl sha256sum makepkg git; do
  command -v "$tool" >/dev/null || { echo "missing: $tool" >&2; exit 1; }
done

deb_url="$REPO_URL/releases/download/$tag/DJ_USB_Tkit_${version}_amd64.deb"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Downloading $deb_url"
curl -fL --retry 3 -o "$work/app.deb" "$deb_url"
sha256="$(sha256sum "$work/app.deb" | cut -d' ' -f1)"

if [[ ! -d "$aur_dir/.git" ]]; then
  echo "Cloning $AUR_URL"
  git clone "$AUR_URL" "$aur_dir"
fi
git -C "$aur_dir" pull --ff-only 2>/dev/null || true

sed -e "s/@PKGVER@/$version/" -e "s/@PKGREL@/$pkgrel/" -e "s/@SHA256@/$sha256/" \
  "$template" > "$aur_dir/PKGBUILD"
(cd "$aur_dir" && makepkg --printsrcinfo > .SRCINFO)

# Test build in a scratch copy, so the AUR clone holds only PKGBUILD/.SRCINFO.
build="$work/build"
mkdir "$build"
cp "$aur_dir/PKGBUILD" "$build/"
cp "$work/app.deb" "$build/dj-usb-tkit-${version}.deb"
(cd "$build" && makepkg -f --noconfirm)
pkgfile="$(find "$build" -maxdepth 1 -name "${PKGNAME}-${version}-${pkgrel}-*.pkg.tar.*" | head -n1)"
echo "Built $(basename "$pkgfile")"

if command -v namcap >/dev/null; then
  namcap "$build/PKGBUILD" "$pkgfile" || true
else
  echo "(namcap not installed; skipping lint)"
fi

if (( install )); then
  sudo pacman -U --noconfirm "$pkgfile"
fi

git -C "$aur_dir" add PKGBUILD .SRCINFO
git -C "$aur_dir" status --short
cat <<EOF

Staged in $aur_dir. To publish:
  git -C "$aur_dir" commit -m "Update to $version-$pkgrel"
  git -C "$aur_dir" push origin HEAD:master
EOF
