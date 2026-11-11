#!/bin/sh
# Builds soundhold and soundhold-window .rpm packages in target/rpm/RPMS. Needs rpm-build.
set -eu

root=$(dirname "$(dirname "$(readlink -f "$0")")")
cd "$root"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/soundhold/Cargo.toml | head -1)

RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo build --release --locked
rpmbuild -bb packaging/soundhold.spec --define "_topdir $root/target/rpm" --define "app_version $version" --define "source_root $root"
