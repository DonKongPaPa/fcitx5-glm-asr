# Installation Guide

## System Requirements

- Linux x86_64
- **Wayland** session (X11 does not support overlay)
- **fcitx5 >= 5.1**
- **PipeWire** (audio recording backend)
- **Wayland compositor with `wlr-layer-shell-unstable-v1` protocol support** (KDE Plasma 6, Sway, Hyprland; GNOME/Mutter does not support layer-shell — disable Use Overlay and use candidate window instead)
- Network connection (to call GLM ASR API)

> **Multi-monitor note**: The overlay works on all compositors supporting layer-shell. Auto-follow active display is **KDE Plasma 6** only. On other desktops the overlay appears on the default monitor. GNOME users should disable **Use Overlay** in the configuration.

---

## Arch Linux

### Build from PKGBUILD

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr
makepkg -si
```

### Install prebuilt package (v0.2.0+)

Download `.pkg.tar.zst` from [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases):

```bash
sudo pacman -U fcitx5-glm-asr-*.pkg.tar.zst
```

### Enable

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
# Restart fcitx5 (KDE will auto-restart it)
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## Debian / Ubuntu

### Install build dependencies

```bash
sudo apt install build-essential cmake g++ pkg-config \
    extra-cmake-modules wayland-protocols git \
    libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev \
    libasound2-dev libwayland-dev libxkbcommon-dev \
    libpipewire-0.3-dev
```

> **Note**: This project uses Cargo.lock v4, which requires Rust 1.87+. The system Rust on Debian stable may be too old. Install the latest version via [rustup](https://rustup.rs/).

### Build and install

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# Build daemon
cd daemon
cargo build --release --locked --features vello-renderer
cd ..

# Build plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

```bash
# Install
cd /path/to/fcitx5-glm-asr
sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### Install prebuilt package (v0.2.0+)

Download `.deb` from [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases):

```bash
sudo dpkg -i fcitx5-glm-asr_*.deb
```

### Enable

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## Fedora / RHEL

### Recommended versions

Fedora 39+ / RHEL 9+ (requires fcitx5 >= 5.1)

### Install build dependencies

```bash
sudo dnf install cmake gcc-c++ pkgconf-pkg-config \
    extra-cmake-modules wayland-protocols-devel git \
    fcitx5-devel alsa-lib-devel wayland-devel libxkbcommon-devel \
    pipewire-devel
```

> **Note**: This project uses Cargo.lock v4, which requires Rust 1.87+. The system Rust on Fedora may be too old. Install the latest version via [rustup](https://rustup.rs/).

### Build and install

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# Build daemon
cd daemon
cargo build --release --locked --features vello-renderer
cd ..

# Build plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

```bash
# Install
cd /path/to/fcitx5-glm-asr
sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### Install prebuilt package (v0.2.0+)

Download `.rpm` from [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases):

```bash
sudo dnf install fcitx5-glm-asr-*.rpm
```

### Enable

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## Universal tarball (v0.2.0+)

Download `.tar.gz` from [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases):

```bash
tar xzf fcitx5-glm-asr-*.tar.gz
cd fcitx5-glm-asr-*/
sudo ./install.sh
```

---

## Configuration

See [README.md](README.md#configuration).

---

## Uninstall

### Arch Linux

```bash
sudo pacman -R fcitx5-glm-asr
```

### Debian / Fedora / Universal tarball

```bash
systemctl --user disable --now glm-asrd

sudo rm -f /usr/bin/glm-asrd
sudo rm -f /usr/lib/systemd/user/glm-asrd.service
sudo rm -f /usr/share/fcitx5/addon/glm-asr.conf
sudo rm -f $(find /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu \
    -path '*/fcitx5/glm-asr.so' 2>/dev/null)

# Optional: remove user config
rm -rf ~/.config/glm-asrd
rm -f ~/.config/fcitx5/conf/glm-asr.conf

# Restart fcitx5
kill $(pgrep -f '/usr/bin/fcitx5$')
```
