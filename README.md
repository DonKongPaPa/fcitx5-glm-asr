# fcitx5-glm-asr

An fcitx5 plugin for voice typing using ZhiPu GLM ASR.

Press and hold a hotkey (default: Right Ctrl) to record, release to transcribe and type.

## Features

- Press-and-hold hotkey (default Right Ctrl) to record, release to transcribe (supports Hold and Toggle modes)
- **Overlay real-time feedback**: progress ring, countdown, waveform visualization during recording; result text displayed after recognition
- **Waveform visualization**: peak-envelope-based real-time waveform with auto-normalization + temporal smoothing
- **Dual rendering backends**: Software (CPU) and Vello (GPU, experimental), switchable at runtime via fcitx5-configtool without restart
- **LLM post-processing**: send ASR results to an LLM for intelligent correction and refinement (disabled by default)
- Candidate window for selecting between multiple correction variants
- GUI configuration via `fcitx5-configtool` (API Key, model, sample rate, hotkey, renderer, LLM settings, etc.)
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

## LLM Post-Processing

When enabled, ASR results are sent to a configurable LLM (via OpenAI-compatible API) for intelligent correction. The LLM returns multiple correction variants as candidates:

- **Fluent version** (light correction): fixes homophones and obvious ASR errors, preserves original style
- **Formal version** (deep correction): optimizes word choice, adds punctuation, adjusts word order
- **Intent-driven corrections**: translation (e.g. "translate to English"), number conversion (e.g. "convert to uppercase Chinese numerals")
- **Scene inference**: adjusts correction confidence based on detected context (casual chat, formal document, data entry, search query)

Candidates are displayed with labeled prefixes (`[✓ label]`) in the candidate list. The original ASR text is always included as the last candidate. If LLM fails, the system gracefully falls back to the raw ASR result.

### Prompt System

Prompts are stored in `~/.config/glm-asrd/prompts/` and can be freely edited:

- `main.md` — main system prompt (role, rules, correction levels, scene inference, intent detection)
- `correction_advices/*.md` — individual correction guidance files loaded automatically

Defaults are generated on first use. Use **Reset LLM Prompts** in fcitx5-configtool to restore defaults.

### LLM Configuration

Open `fcitx5-configtool` and configure under **GLM ASR**:

| Setting | Description | Default |
|---------|-------------|---------|
| Enable LLM Post-processing | Enable LLM correction pipeline | Off |
| LLM API Key | API key for LLM (leave empty to use ASR API Key) | *(empty)* |
| LLM API Base URL | OpenAI-compatible API base URL | `https://open.bigmodel.cn/api/paas/v4` |
| LLM Model Name | Model identifier | `glm-4-flash` |
| LLM Timeout | Per-request timeout in seconds (5–120) | 15 |
| Enable LLM Thinking Mode | Support reasoning models (e.g. glm-4.7-flash, DeepSeek) | Off |
| LLM Max Output Tokens | Max output tokens to prevent loops (256–8192) | 2048 |
| Reset LLM Prompts to Default | Check and apply to reset prompts, auto-unchecks after reset | Off |

The model must support JSON structured output (`response_format: json_object`).

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
