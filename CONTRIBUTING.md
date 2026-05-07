# Contributing to fcitx5-glm-asr

## Development Environment

### System Dependencies

#### Arch Linux

```bash
sudo pacman -S --needed \
    base-devel rust cargo cmake extra-cmake-modules pkgconf \
    wayland-protocols git \
    fcitx5 pipewire gcc-libs wayland vulkan-icd-loader alsa-lib
```

#### Debian / Ubuntu

```bash
sudo apt install \
    build-essential rustc cargo cmake extra-cmake-modules pkg-config \
    wayland-protocols git \
    libfcitx5core-dev libfcitx5config-dev libfcitx5utils-dev \
    libasound2-dev libwayland-dev libpipewire-0.3-dev libvulkan-dev
```

#### Fedora / RHEL

```bash
sudo dnf install \
    @development-tools rust cargo cmake extra-cmake-modules pkgconf-pkg-config \
    wayland-protocols git \
    fcitx5-devel alsa-lib-devel wayland-devel pipewire-devel vulkan-loader-devel
```

### Building

#### Quick Start (Makefile)

```bash
make dev                        # Build + install everything (incremental)
make restart                    # Build + install + restart fcitx5 + glm-asrd
make daemon                     # Build Rust daemon only
make plugin                     # Build C++ plugin only
make dev BUILD_TYPE=debug       # Debug build
make uninstall-dev              # Remove dev plugin, revert to system package
make clean                      # Remove build artifacts
```

The Makefile installs:
- Daemon binaries (`glm-asrd`, `glm-asr-overlay`) to `/usr/bin/` (requires sudo)
- Plugin (`glm-asr.so`) to `/usr/lib/fcitx5/` (requires sudo)
- Addon config (`glm-asr.conf`) to `/usr/share/fcitx5/addon/` (requires sudo)

Dev plugin overwrites the system package files, so `make dev` replaces the
installed version. Use `make uninstall-dev` to remove, then reinstall the
system package: `sudo pacman -S fcitx5-glm-asr`.

#### Manual Build

```bash
# Build daemon (with Vello GPU renderer)
cd daemon && cargo build --release --locked --features vello-renderer

# Build fcitx5 plugin
mkdir -p build && cd build
cmake .. -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
make
```

### Local Testing

```bash
# Build, install, and restart everything
make restart

# Run with mock ASR (no API key needed)
systemctl --user set-environment GLM_ASR_MOCK=1
make restart

# Test IPC
python3 -c "
import socket, json
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(5)
sock.connect('/run/user/$(id -u)/glm-asrd.sock')
sock.sendall((json.dumps({'cmd': 'ping'}) + '\n').encode())
print(sock.recv(4096).decode().strip())
sock.close()
"

# Check memory usage
cat /proc/$(pgrep glm-asrd)/smaps_rollup | grep -E "^(Rss|Pss|Private_Dirty):"
```

### Docker Build Verification

Before submitting changes, verify builds pass on all platforms:

```bash
cd docker/arch    && podman compose up   # Arch (makepkg)
cd docker/debian  && podman compose up   # Debian
cd docker/fedora  && podman compose up   # Fedora
```

See [docker/README.md](docker/README.md) for details.

## Project Structure

```
daemon/          Rust daemon (glm-asrd)
  src/
    main.rs        Entry point, overlay thread management
    audio.rs       Recording (cpal/ALSA)
    asr.rs         GLM ASR API client
    ipc.rs         Unix socket IPC (newline-delimited JSON)
    config.rs      Configuration types
    resample.rs    Audio resampling
    overlay/       Wayland overlay (layer-shell)
      mod.rs         Lazy GPU init, Wayland reconnect
      wayland.rs     Surface management
      renderer/      Drawing backends
        mod.rs         OverlayRenderer trait
        software.rs    CPU renderer (tiny-skia + cosmic-text)
        vello.rs       GPU renderer (wgpu + vello, feature-gated)
plugin/          C++ fcitx5 addon
  glm_asr_addon.h
  glm_asr_addon.cpp
  glm-asr-addon.conf.in
data/
  glm-asrd.service   systemd user service
docker/          Multi-platform build verification
  arch/            Arch Linux (makepkg)
  debian/          Debian bookworm
  fedora/          Fedora 41
```

## Commit Conventions

[Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add new feature
fix: fix a bug
chore: maintenance (dependencies, CI, versions)
docs: documentation changes
refactor: code restructuring
```

Tag format: `vMAJOR.MINOR.PATCH` (e.g., `v0.1.2`)

## Known Pitfalls

### makepkg `-flto=auto` breaks ring

Arch Linux `makepkg` appends `LTOFLAGS="-flto=auto"` to `CFLAGS`/`LDFLAGS`.
ring's build script compiles C/ASM code via the `cc` crate, which picks up
`CFLAGS`. With `-flto=auto`, GCC produces LTO bytecode instead of native
objects — the linker then can't resolve symbols like `ring_core_0_17_14__*`.

The PKGBUILD `build()` function strips `-flto=auto` before calling cargo.

### Do NOT restart fcitx5 with `fcitx5 -r`

This causes D-Bus conflicts. The correct way:

```bash
kill $(pgrep -f '/usr/bin/fcitx5$')
# KDE will auto-restart fcitx5
```

Or simply: `fcitx5 -d` (daemonize).

### Stale plugin after `make dev`

`make dev` overwrites system-level plugin files. After `make uninstall-dev`,
reinstall the system package to restore:

```bash
make uninstall-dev
sudo pacman -S fcitx5-glm-asr
```
