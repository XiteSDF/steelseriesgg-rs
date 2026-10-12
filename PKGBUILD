#!/usr/bin/env bash
# Maintainer: Your Name <your.email@example.com>
# shell: bash
# shellcheck disable=SC2034,SC2154
pkgname=ssgg
pkgver=0.3.1 # x-release-please-version
pkgrel=1
pkgdesc="A complete open-source SteelSeries GG replacement for Linux - RGB lighting, audio mixer, and GameSense support"
arch=('x86_64')
url="https://github.com/Ven0m0/steelseriesgg-rs"
license=('MIT')
# hidapi is not a dependency: the Linux build uses hidapi's pure-Rust hidraw backend.
depends=('glibc' 'gcc-libs' 'systemd' 'webkit2gtk-4.1' 'gtk3')
makedepends=('rust')
optdepends=(
  'pipewire: Sonar-style audio mixer (virtual Game/Chat/Media/Aux/Mic devices)'
  'pipewire-pulse: provides pactl-compatible control used by the mixer'
  'libpulse: pactl command used by the mixer'
  'wireplumber: wpctl volume control used by the mixer'
  'noise-suppression-for-voice: RNNoise microphone noise suppression'
  'gpu-screen-recorder: Moments (instant replay clips)'
  'playerctl: now-playing screen on OLED keyboards'
  'libnotify: desktop notifications (battery low, profile switched)'
  'xdg-utils: open the control panel in a browser'
)
install=ssgg.install
source=(
  "$pkgname-$pkgver.tar.gz::$url/archive/refs/tags/v$pkgver.tar.gz"
)
# Update this when creating a release
sha256sums=('SKIP')

_archive_dir="steelseriesgg-rs-$pkgver"
_rust_target="$CARCH-unknown-linux-gnu"

# NOTE: $srcdir is only populated when makepkg invokes the build functions, not
# while the PKGBUILD is sourced. Deriving CARGO_HOME/CARGO_TARGET_DIR at parse
# time would expand to /cargo-home and /target, which the unprivileged build
# user cannot write to. Reference $srcdir inside the functions instead.

_resolve_archive_dir() {
  local candidate
  for candidate in \
    "$srcdir/$_archive_dir" \
    "$srcdir/$pkgname-$pkgver" \
    "$srcdir/${url##*/}-$pkgver"
  do
    if [[ -d "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  printf 'Missing extracted source directory in %s\n' "$srcdir" >&2
  return 1
}

prepare() {
  cd "$(_resolve_archive_dir)" || return 1
  export CARGO_HOME="$srcdir/cargo-home"
  cargo fetch --locked --target "$_rust_target"
}

build() {
  cd "$(_resolve_archive_dir)" || return 1
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR="$srcdir/target"
  cargo build -r --frozen --bin ssgg
  cargo build -r --frozen --features gui --bin ssgg-gui
}

check() {
  cd "$(_resolve_archive_dir)" || return 1
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR="$srcdir/target"
  cargo test --frozen
}

package() {
  cd "$(_resolve_archive_dir)" || return 1
  install -Dm755 "$srcdir/target/release/ssgg" "$pkgdir/usr/bin/ssgg"
  install -Dm755 "$srcdir/target/release/ssgg-gui" "$pkgdir/usr/bin/ssgg-gui"
  install -Dm644 assets/ssgg.service "$pkgdir/usr/lib/systemd/user/ssgg.service"
  install -Dm644 assets/70-steelseries.rules "$pkgdir/usr/lib/udev/rules.d/70-steelseries.rules"
  install -Dm644 assets/ssgg-uinput.conf "$pkgdir/usr/lib/modules-load.d/ssgg-uinput.conf"
  install -Dm644 assets/ssgg.desktop "$pkgdir/usr/share/applications/ssgg.desktop"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}
