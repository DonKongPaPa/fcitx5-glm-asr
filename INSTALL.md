# 编译安装指南

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

### 使用 PKGBUILD 构建

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr
makepkg -si
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
# 重启 fcitx5（KDE 会自动拉起）
kill $(pgrep -f '/usr/bin/fcitx5$')
```

### 配置

打开 `fcitx5-configtool`，在插件管理中找到 **GLM ASR**，配置以下参数：

- **API Key** — 智谱开放平台 API Key（必填）
- **Model** — ASR 模型名称（默认 `glm-asr-2512`）
- **API URL** — API 端点地址（默认 `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`，一般无需修改）
- **Sample Rate** — 录音采样率（默认 16000 Hz）
- **Use Overlay** — 启用 overlay 实时反馈（默认开启）

---

## Debian / Ubuntu

### 安装编译依赖

```bash
sudo apt install rustc cargo cmake g++ pkg-config \
  libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev \
  libasound2-dev libwayland-dev
```

### 编译

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# 编译 daemon
cd daemon
cargo build --release
cd ..

# 编译 plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

### 安装

```bash
cd /path/to/fcitx5-glm-asr

sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

### 配置

打开 `fcitx5-configtool`，在插件管理中找到 **GLM ASR**，配置以下参数：

- **API Key** — 智谱开放平台 API Key（必填）
- **Model** — ASR 模型名称（默认 `glm-asr-2512`）
- **API URL** — API 端点地址（默认 `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`，一般无需修改）
- **Sample Rate** — 录音采样率（默认 16000 Hz）
- **Use Overlay** — 启用 overlay 实时反馈（默认开启）

---

## Fedora / RHEL

### 推荐系统版本

Fedora 39+ / RHEL 9+（需要 fcitx5 >= 5.1）

### 安装编译依赖

```bash
sudo dnf install rust cargo cmake gcc-c++ pkgconf-pkg-config \
  fcitx5-devel alsa-lib-devel wayland-devel
```

### 编译

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr

# 编译 daemon
cd daemon
cargo build --release
cd ..

# 编译 plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

### 安装

```bash
cd /path/to/fcitx5-glm-asr

sudo install -Dm755 daemon/target/release/glm-asrd /usr/bin/glm-asrd
sudo make -C build install
sudo install -Dm644 data/glm-asrd.service /usr/lib/systemd/user/glm-asrd.service
```

### 启用

```bash
systemctl --user daemon-reload
systemctl --user enable --now glm-asrd
kill $(pgrep -f '/usr/bin/fcitx5$')
```

### 配置

打开 `fcitx5-configtool`，在插件管理中找到 **GLM ASR**，配置以下参数：

- **API Key** — 智谱开放平台 API Key（必填）
- **Model** — ASR 模型名称（默认 `glm-asr-2512`）
- **API URL** — API 端点地址（默认 `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`，一般无需修改）
- **Sample Rate** — 录音采样率（默认 16000 Hz）
- **Use Overlay** — 启用 overlay 实时反馈（默认开启）

---

## 卸载

### Arch Linux（通过 pacman）

```bash
sudo pacman -R fcitx5-glm-asr
```

### Debian / Fedora（手动编译安装）

```bash
# 停止服务
systemctl --user disable --now glm-asrd

# 删除已安装文件
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
