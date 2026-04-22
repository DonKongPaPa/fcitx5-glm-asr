# fcitx5-glm-asr

基于智谱 GLM ASR 的 Linux 语音输入 fcitx5 插件。

长按快捷键（默认右 Ctrl）录音，松开后自动识别并输入文字。

## 功能

- 长按快捷键（默认右 Ctrl）录音，松开识别（支持长按/切换两种模式）
- **Overlay 实时反馈**：录音时显示进度环、倒计时、波形可视化；识别完成后显示结果文字
- **波形可视化**：基于峰值包络的实时波形，自动归一化 + 时间平滑，直观展示说话状态
- **双渲染后端**：Software（CPU）和 Vello（GPU 实验性），可通过 fcitx5-configtool 运行时切换，无需重启
- 候选框状态反馈（关闭 overlay 时的回退方案）
- fcitx5-configtool 图形化配置（API Key、模型、采样率、快捷键、渲染器等）
- 多显示器支持（自动跟随当前活跃显示器显示 overlay）

## 前置依赖

### 必需

- **Linux x86_64** + **Wayland** 会话
- **fcitx5 >= 5.1**
- **PipeWire**（录音后端）
- **Wayland compositor 需支持 `wlr-layer-shell-unstable-v1` 协议**（overlay 显示必需）
  - KDE Plasma 6（KWin）✅
  - Sway / Hyprland / wlroots 系 ✅
  - GNOME (Mutter) — 不支持 layer-shell，需在配置中关闭 Use Overlay，改用候选框反馈
- 网络连接（调用 GLM ASR API）

### 编译依赖

- Rust toolchain（rustc + cargo）
- C++ 编译器（GCC 或 Clang）
- CMake >= 3.16 + extra-cmake-modules
- pkgconf / pkg-config
- Wayland 开发库（libwayland）
- fcitx5 开发库（libfcitx5core 等）

### 多显示器说明

overlay 在所有支持 layer-shell 的 compositor（KDE Plasma 6、Sway、Hyprland 等）上均可正常使用。

其中**自动跟随当前活跃显示器**的功能需要 KDE Plasma 6（通过 D-Bus 查询）。其他桌面环境中 overlay 显示在默认/主显示器上，不影响录音识别功能。

GNOME (Mutter) 不支持 layer-shell 协议，overlay 无法显示，请在配置中关闭 **Use Overlay** 选项，插件会自动回退到 fcitx5 候选框显示状态。

## 安装

详见 [INSTALL.md](INSTALL.md)

## 配置

使用 `fcitx5-configtool` 打开插件管理，找到 **GLM ASR**，配置以下参数：

- **API Key** — 智谱开放平台 API Key（必填）
- **Model** — ASR 模型名称（默认 `glm-asr-2512`）
- **API URL** — API 端点地址（默认 `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`，一般无需修改）
- **Sample Rate** — 录音采样率（默认 16000 Hz）
- **Trigger Key** — 触发快捷键（默认右 Ctrl）
- **Trigger Mode** — 触发模式：长按（Hold）或 按下切换（Toggle）
- **Use Overlay** — 启用 overlay 实时反馈（默认开启）。关闭后回退到 fcitx5 候选框显示状态
- **Overlay Renderer** — Overlay 渲染后端（默认 Software）
  - `Software` — CPU 渲染，无额外依赖
  - `Vello (Experimental)` — GPU 渲染（需要 Vulkan 兼容 GPU），通过 `--features vello-renderer` 编译启用

## 许可证

[MIT](LICENSE)
