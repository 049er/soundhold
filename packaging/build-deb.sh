#!/bin/sh
# Builds soundhold and soundhold-window .deb packages in target/deb. Needs dpkg-dev.
set -eu

root=$(dirname "$(dirname "$(readlink -f "$0")")")
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/soundhold/Cargo.toml | head -1)
arch=$(dpkg --print-architecture)
out=target/deb
work=$out/work

RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo build --release

rm -rf "$work"
mkdir -p "$work/debian"
printf 'Source: soundhold\n\nPackage: soundhold\nArchitecture: any\n' > "$work/debian/control"

shared_libraries() {
    (cd "$work" && dpkg-shlibdeps -O --warnings=0 "../../release/$1") | sed 's/^shlibs:Depends=//'
}

write_control() {
    mkdir -p "$1/DEBIAN"
    cat > "$1/DEBIAN/control" <<CONTROL
Package: $2
Version: $version
Architecture: $arch
Maintainer: Soundhold <https://github.com/049er/soundhold>
Section: sound
Priority: optional
Homepage: https://github.com/049er/soundhold
Depends: $3
$4Description: $5
CONTROL
}

copy_docs() {
    install -Dm644 packaging/copyright "$1/usr/share/doc/$2/copyright"
}

daemon=$work/soundhold
install -Dm755 target/release/soundhold "$daemon/usr/bin/soundhold"
install -Dm644 -t "$daemon/usr/share/soundhold" sofa/*.sofa
install -Dm644 70-soundhold.rules "$daemon/usr/lib/udev/rules.d/70-soundhold.rules"
install -Dm644 data/icons/hicolor/scalable/apps/soundhold.svg "$daemon/usr/share/icons/hicolor/scalable/apps/soundhold.svg"
install -Dm644 data/icons/hicolor/symbolic/apps/soundhold-symbolic.svg "$daemon/usr/share/icons/hicolor/symbolic/apps/soundhold-symbolic.svg"
copy_docs "$daemon" soundhold
depends="$(shared_libraries soundhold), pipewire-bin, libpipewire-0.3-modules, libspa-0.2-modules-extra, wireplumber, bluez, systemd"
write_control "$daemon" soundhold "$depends" "Recommends: soundhold-window
" "Head tracked spatial sound for Bluetooth headphones"
install -m755 packaging/postinst "$daemon/DEBIAN/postinst"
install -m755 packaging/postrm "$daemon/DEBIAN/postrm"

window=$work/soundhold-window
install -Dm755 target/release/soundhold-window "$window/usr/bin/soundhold-window"
install -Dm644 packaging/io.github.soundhold.Window.desktop "$window/usr/share/applications/io.github.soundhold.Window.desktop"
copy_docs "$window" soundhold-window
write_control "$window" soundhold-window "$(shared_libraries soundhold-window), soundhold (= $version)" "" "Window for Soundhold"

for package in soundhold soundhold-window; do
    dpkg-deb --root-owner-group --build "$work/$package" "$out/${package}_${version}_${arch}.deb"
done
