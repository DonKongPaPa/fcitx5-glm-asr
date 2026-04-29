# Maintainer: DonKongPaPa
pkgname=fcitx5-glm-asr
pkgver=0.2.2
pkgrel=1
pkgdesc="Voice typing using GLM ASR model - fcitx5 plugin with Rust daemon"
arch=('x86_64')
url="https://github.com/DonKongPaPa/fcitx5-glm-asr"
license=('MIT')
depends=('fcitx5' 'pipewire' 'gcc-libs' 'wayland' 'vulkan-icd-loader' 'alsa-lib')
makedepends=('rust' 'cargo' 'cmake' 'extra-cmake-modules' 'pkgconf' 'wayland-protocols' 'git')
source=("git+${url}.git#tag=v$pkgver")
sha256sums=('SKIP')

prepare() {
    cd "$srcdir/$pkgname/daemon"
    cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
    cd "$srcdir/$pkgname"

    cd daemon
    _cargo_cflags="${CFLAGS//-flto=auto/}"
    _cargo_ldflags="${LDFLAGS//-flto=auto/}"
    CFLAGS="$_cargo_cflags" LDFLAGS="$_cargo_ldflags" \
        cargo build --release --locked --features vello-renderer
    cd ..

    mkdir -p build
    cmake -S . -B build \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/usr
    make -C build
}

package() {
    cd "$srcdir/$pkgname"

    install -Dm755 "daemon/target/release/glm-asrd" "$pkgdir/usr/bin/glm-asrd"
    install -Dm644 "data/glm-asrd.service" "$pkgdir/usr/lib/systemd/user/glm-asrd.service"
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    make -C build DESTDIR="$pkgdir" install
}
