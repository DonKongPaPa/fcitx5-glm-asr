# fcitx5-glm-asr

An fcitx5 plugin for voice typing using ZhiPu GLM ASR.

Press and hold a hotkey (default: Right Ctrl) to record, release to transcribe and type.

## Features

- Press-and-hold hotkey (default Right Ctrl) to record, release to transcribe (supports Hold and Toggle modes)
- **Overlay real-time feedback**: progress ring, countdown, waveform visualization during recording; result text displayed after recognition
- **Waveform visualization**: peak-envelope-based real-time waveform with auto-normalization + temporal smoothing
- **Dual rendering backends**: Software (CPU) and Vello (GPU, experimental), switchable at runtime via fcitx5-configtool without restart
- Candidate window status feedback (fallback when overlay is disabled)
- GUI configuration via `fcitx5-configtool` (API Key, model, sample rate, hotkey, renderer, etc.)
- Multi-monitor support (overlay follows the active display)

## Quick Start (Arch Linux)

```bash
git clone https://github.com/DonKongPaPa/fcitx5-glm-asr.git
cd fcitx5-glm-asr
makepkg -si
```

For other platforms, see [INSTALL.md](INSTALL.md).

## Requirements

- **Linux x86_64** + **Wayland** session
- **fcitx5 >= 5.1**
- **PipeWire** (audio recording backend)
- **ALSA** (audio input, usually pre-installed)
- **Wayland compositor with `wlr-layer-shell-unstable-v1` protocol support** (required for overlay)
  - KDE Plasma 6 (KWin)
  - Sway / Hyprland / wlroots-based compositors
  - GNOME (Mutter) — does not support layer-shell; disable Use Overlay and use candidate window instead
- Network connection (to call GLM ASR API)

## Configuration

Open `fcitx5-configtool`, find **GLM ASR** in the addon manager, and configure:

- **API Key** — ZhiPu Open Platform API key (required)
- **Model** — ASR model name (default: `glm-asr-2512`)
- **API URL** — API endpoint (default: `https://open.bigmodel.cn/api/paas/v4/audio/transcriptions`, usually no need to change)
- **Sample Rate** — Recording sample rate (default: 16000 Hz)
- **Trigger Key** — Hotkey to trigger recording (default: Right Ctrl)
- **Trigger Mode** — Trigger mode: Hold (press and hold) or Toggle (press to start/stop)
- **Use Overlay** — Enable overlay real-time feedback (default: on). Falls back to fcitx5 candidate window when disabled
- **Overlay Renderer** — Overlay rendering backend (default: Software)
  - `Software` — CPU rendering, no extra dependencies
  - `Vello (Experimental)` — GPU rendering (requires Vulkan-compatible GPU), compile with `--features vello-renderer`

## Multi-Monitor Notes

The overlay works on all compositors supporting layer-shell (KDE Plasma 6, Sway, Hyprland, etc.).

**Auto-follow active display** requires KDE Plasma 6 (via D-Bus query). On other desktops, the overlay appears on the default/primary monitor. Recording and recognition are not affected.

GNOME (Mutter) does not support the layer-shell protocol. Disable **Use Overlay** in the configuration and the plugin will fall back to the fcitx5 candidate window.

## Documentation

- [Installation Guide](INSTALL.md) — Arch / Debian / Fedora / Universal tarball
- [Contributing Guide](CONTRIBUTING.md) — Dev setup, Docker build verification, commit conventions
- [Release Process](RELEASING.md) — Maintainer reference

## License

[MIT](LICENSE)
