# 安装指南

## 系统要求

- Linux x86_64
- **Wayland** 会话（X11 不支持 overlay）
- **fcitx5 >= 5.1**
- **PipeWire**（录音后端）
- **Wayland compositor 需支持 `wlr-layer-shell-unstable-v1` 协议**（KDE Plasma 6、Sway、Hyprland 等支持；GNOME/Mutter 不支持 layer-shell，需关闭 Use Overlay 改用候选框）
- 网络连接（调用 GLM ASR API）

> **多显示器提示**：overlay 在所有支持 layer-shell 的 compositor 上均可正常使用。其中自动跟随当前活跃显示器仅 **KDE Plasma 6** 支持，其他桌面环境中 overlay 显示在默认显示器上。GNOME 用户请在配置中关闭 **Use Overlay** 选项。

---

## Arch Linux

### PKGBUILD 编译安装

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr
makepkg -si
```

### 预编译包安装（v0.2.0+）

从 [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases) 下载 `.pkg.tar.zst`：

```bash
sudo pacman -U fcitx5-glm-asr-*.pkg.tar.zst
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
# 重启 fcitx5（KDE 会自动拉起）
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## Debian / Ubuntu

### 安装编译依赖

```bash
sudo apt install build-essential cmake g++ pkg-config \
    extra-cmake-modules wayland-protocols git \
    libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev \
    libasound2-dev libwayland-dev libxkbcommon-dev \
    libpipewire-0.3-dev
```

> **注意**：项目使用 Cargo.lock v4，需要 Rust 1.87+。Debian stable 的系统 Rust 可能过旧，建议通过 [rustup](https://rustup.rs/) 安装最新版。

### 编译安装

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# 编译 daemon
cd daemon
cargo build --release --locked --features vello-renderer
cd ..

# 编译 plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

```bash
# 安装
cd /path/to/fcitx5-glm-asr
sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### 预编译包安装（v0.2.0+）

从 [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases) 下载 `.deb`：

```bash
sudo dpkg -i fcitx5-glm-asr_*.deb
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## Fedora / RHEL

### 推荐系统版本

Fedora 39+ / RHEL 9+（需要 fcitx5 >= 5.1）

### 安装编译依赖

```bash
sudo dnf install cmake gcc-c++ pkgconf-pkg-config \
    extra-cmake-modules wayland-protocols-devel git \
    fcitx5-devel alsa-lib-devel wayland-devel libxkbcommon-devel \
    pipewire-devel
```

> **注意**：项目使用 Cargo.lock v4，需要 Rust 1.87+。Fedora 系统包可能版本不够，建议通过 [rustup](https://rustup.rs/) 安装最新版。

### 编译安装

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# 编译 daemon
cd daemon
cargo build --release --locked --features vello-renderer
cd ..

# 编译 plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

```bash
# 安装
cd /path/to/fcitx5-glm-asr
sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### 预编译包安装（v0.2.0+）

从 [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases) 下载 `.rpm`：

```bash
sudo dnf install fcitx5-glm-asr-*.rpm
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

---

## 通用 tarball（v0.2.0+）

从 [GitHub Releases](https://github.com/DonKongPaPa/fcitx5-glm-asr/releases) 下载 `.tar.gz`：

```bash
tar xzf fcitx5-glm-asr-*.tar.gz
cd fcitx5-glm-asr-*/
sudo ./install.sh
```

---

## 配置

详见 [README.md](README.md#配置)。

---

## 卸载

### Arch Linux

```bash
sudo pacman -R fcitx5-glm-asr
```

### Debian / Fedora / 通用 tarball

```bash
systemctl --user disable --now glm-asrd

sudo rm -f /usr/bin/glm-asrd
sudo rm -f /usr/lib/systemd/user/glm-asrd.service
sudo rm -f /usr/share/fcitx5/addon/glm-asr.conf
sudo rm -f $(find /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu \
    -path '*/fcitx5/glm-asr.so' 2>/dev/null)

# 可选：删除用户配置
rm -rf ~/.config/glm-asrd
rm -f ~/.config/fcitx5/conf/glm-asr.conf

# 重启 fcitx5
kill $(pgrep -f '/usr/bin/fcitx5$')
```
