# Maintainer: DonKongPaPa
pkgname=fcitx5-glm-asr
pkgver=0.1.0
pkgrel=1
pkgdesc="Voice typing using GLM ASR model - fcitx5 plugin with Rust daemon"
arch=('x86_64')
url="https://github.com/DonKongPaPa/fcitx5-glm-asr"
license=('MIT')
depends=('fcitx5' 'pipewire' 'gcc-libs' 'wayland')
makedepends=('rust' 'cargo' 'cmake' 'extra-cmake-modules' 'pkgconf' 'wayland-protocols')
source=("$pkgname-$pkgver.tar.gz::https://github.com/DonKongPaPa/fcitx5-glm-asr/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname-$pkgver"

    cd daemon
    cargo build --release --locked
    cd ..

    mkdir -p build
    cmake -S . -B build \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/usr
    make -C build
}

package() {
    cd "$srcdir/$pkgname-$pkgver"

    install -Dm755 "daemon/target/release/glm-asrd" "$pkgdir/usr/bin/glm-asrd"
    install -Dm644 "data/glm-asrd.service" "$pkgdir/usr/lib/systemd/user/glm-asrd.service"
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    make -C build DESTDIR="$pkgdir" install
}
