#!/usr/bin/env bash
# Fabrique les deux versions Linux de Doll//Wave, dans target/linux/ :
# - dollwave_<version>_amd64.deb : s'installe comme un vrai programme sur
#   Ubuntu, Debian, Mint… (menu des applications, « Ouvrir avec »).
# - Doll-Wave-<version>-x86_64.AppImage : un seul fichier, sans installation,
#   pour presque toutes les distributions.
#
# Il faut avoir lancé `cargo build --release` avant, et installé cargo-deb.
# appimagetool est téléchargé s'il n'est pas déjà là.
set -euo pipefail
cd "$(dirname "$0")/../.."

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
sortie=target/linux
mkdir -p "$sortie"

# Le .deb.
cargo deb --no-build --output "$sortie/dollwave_${version}_amd64.deb"

# L'AppImage : un dossier « AppDir » rangé comme un petit système, puis
# appimagetool le transforme en un seul fichier.
appdir=target/linux/AppDir
rm -rf "$appdir"
install -Dm755 target/release/dollwave "$appdir/usr/bin/dollwave"
install -Dm644 installateur/linux/dollwave.desktop "$appdir/usr/share/applications/dollwave.desktop"
install -Dm644 assets/icone/icone-256.png "$appdir/usr/share/icons/hicolor/256x256/apps/dollwave.png"
install -Dm644 assets/icone/icone.svg "$appdir/usr/share/icons/hicolor/scalable/apps/dollwave.svg"
cp installateur/linux/dollwave.desktop "$appdir/dollwave.desktop"
cp assets/icone/icone-256.png "$appdir/dollwave.png"
ln -s usr/bin/dollwave "$appdir/AppRun"

outil=target/linux/appimagetool
if [ ! -x "$outil" ]; then
    curl -fsSL -o "$outil" \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x "$outil"
fi
# Sans FUSE (machines de compilation, conteneurs), l'outil se lance quand même.
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$outil" --no-appstream "$appdir" \
    "$sortie/Doll-Wave-${version}-x86_64.AppImage"

ls -l "$sortie"/*.deb "$sortie"/*.AppImage
