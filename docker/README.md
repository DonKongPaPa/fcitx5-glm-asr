# Docker Build Verification

Multi-platform build verification using Podman containers. Ensures reproducible
builds across Arch Linux, Debian, and Fedora.

## Why Docker Verification?

Local builds can succeed while `makepkg` fails due to environment differences.
The most notable issue: **Arch Linux `makepkg` appends `-flto=auto` (from
`LTOFLAGS` in `/etc/makepkg.conf`) to `CFLAGS` and `LDFLAGS`**. This causes
ring's C/ASM code to compile as GCC LTO bytecode instead of native machine
code, producing "undefined symbol" errors at link time.

Docker containers simulate a clean environment to catch these issues before
release.

## Directory Structure

```
docker/
├── README.md              This file
├── arch/
│   ├── Dockerfile         Arch Linux (makepkg)
│   └── docker-compose.yml
├── debian/
│   ├── Dockerfile         Debian trixie (cargo + cmake)
│   └── docker-compose.yml
└── fedora/
    ├── Dockerfile         Fedora latest (cargo + cmake)
    └── docker-compose.yml
```

## Quick Start

Build and test all platforms:

```bash
# Arch Linux (full makepkg)
cd docker/arch && podman compose up

# Debian (cargo + cmake)
cd docker/debian && podman compose up

# Fedora (cargo + cmake)
cd docker/fedora && podman compose up
```

Rebuild images (after dependency changes):

```bash
podman compose build --no-cache
```

Interactive debugging:

```bash
podman compose run --rm build-test bash
```

## Platform Details

### Arch Linux (`docker/arch/`)

- **Base image**: `archlinux:latest`
- **What it tests**: Full `makepkg -sf` pipeline (fetch, build, package)
- **Key verification**: PKGBUILD correctness, `cargo fetch --locked`,
  `-flto=auto` stripping
- **Does NOT install**: `go`, `nasm` — verifies ring TLS has zero external
  build dependencies

### Debian (`docker/debian/`)

- **Base image**: `debian:trixie` (testing, for Rust 1.87+ Cargo.lock v4 support)
- **What it tests**: `cargo build --release --locked --features vello-renderer`
  + `cmake` + `make`
- **Key verification**: Dependency resolution on Debian/Ubuntu

### Fedora (`docker/fedora/`)

- **Base image**: `fedora:latest`
- **What it tests**: Same as Debian (cargo + cmake)
- **Key verification**: Dependency resolution on Fedora/RHEL

## Mirror Configuration

Docker builds in China may be slow or fail due to network issues. Configure
mirrors for each layer: base images, package managers, and Rust toolchain.

### Docker Hub / Base Images

Configure Podman registry mirror in `/etc/containers/registries.conf`:

```toml
[[registry]]
prefix = "docker.io"
location = "docker.io"

[[registry.mirror]]
location = "mirror.ccs.tencentyun.com"
# Or other mirrors:
# location = "docker.mirrors.ustc.edu.cn"
# location = "hub-mirror.c.163.com"
```

### Arch Linux (pacman)

Arch Dockerfiles use the default mirror. To use a faster mirror, add a `RUN`
before `pacman -Syu`:

```dockerfile
RUN echo 'Server = https://mirrors.tuna.tsinghua.edu.cn/archlinux/$repo/os/$arch' \
    > /etc/pacman.d/mirrorlist
```

Common mirrors:
- Tsinghua: `https://mirrors.tuna.tsinghua.edu.cn/archlinux/$repo/os/$arch`
- USTC: `https://mirrors.ustc.edu.cn/archlinux/$repo/os/$arch`
- Tencent: `https://mirrors.cloud.tencent.com/archlinux/$repo/os/$arch`

### Debian (apt)

Replace the default sources in Dockerfile:

```dockerfile
RUN sed -i 's|deb.debian.org|mirrors.ustc.edu.cn|g' /etc/apt/sources.list.d/debian.sources
```

Common mirrors:
- USTC: `mirrors.ustc.edu.cn`
- Tsinghua: `mirrors.tuna.tsinghua.edu.cn`
- Tencent: `mirrors.cloud.tencent.com`
- Aliyun: `mirrors.aliyun.com`

### Fedora (dnf)

Fedora uses metalink by default (auto-selects nearest mirror). If metalink is
unreliable, replace with baseurl:

```dockerfile
RUN sed -i 's|^metalink=|#metalink=|g; \
    s|^#baseurl=http://download.example/pub/fedora/linux|baseurl=https://mirrors.ustc.edu.cn/fedora|g' \
    /etc/yum.repos.d/fedora.repo /etc/yum.repos.d/fedora-updates.repo && \
    rm -f /etc/yum.repos.d/fedora-cisco-openh264.repo
```

Common mirrors:
- USTC: `https://mirrors.ustc.edu.cn/fedora`
- Tsinghua: `https://mirrors.tuna.tsinghua.edu.cn/fedora`
- Tencent: `https://mirrors.cloud.tencent.com/fedora`

> **Note**: Remove `fedora-cisco-openh264.repo` to avoid errors — this repo
> has no mirror and is not needed for builds.

### Rustup / Rust Toolchain

Debian and Fedora Dockerfiles use rustup to install Rust 1.87+ (required for
Cargo.lock v4). Set these environment variables **before** running rustup:

```dockerfile
ENV RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
ENV RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup
```

Download the init binary from the mirror as well:

```dockerfile
RUN curl -k -L -o /tmp/rustup-init \
    https://mirrors.ustc.edu.cn/rust-static/rustup/dist/x86_64-unknown-linux-gnu/rustup-init && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y && \
    rm /tmp/rustup-init
```

Common rustup mirrors:
- USTC: `https://mirrors.ustc.edu.cn/rust-static`
- Tsinghua: `https://mirrors.tuna.tsinghua.edu.cn/rustup`
- ByteDance: `https://rsproxy.cn`

## Common Issues

### `undefined symbol: ring_core_0_17_14__*` (Arch only)

**Cause**: makepkg appends `-flto=auto` to `CFLAGS`. ring's C code gets
compiled as GCC LTO bytecode.

**Fix**: PKGBUILD `build()` strips `-flto=auto` before calling cargo:

```bash
_cargo_cflags="${CFLAGS//-flto=auto/}"
_cargo_ldflags="${LDFLAGS//-flto=auto/}"
CFLAGS="$_cargo_cflags" LDFLAGS="$_cargo_ldflags" cargo build ...
```

### `Package alsa was not found` (all platforms)

**Cause**: Missing `alsa-lib` / `libasound2-dev` / `alsa-lib-devel`.

**Fix**: Ensure the Dockerfile installs the ALSA development package for the
platform.

### `cargo fetch --locked` fails

**Cause**: `Cargo.lock` is out of sync with `Cargo.toml` (e.g., version bump
without regenerating lock file).

**Fix**: Run `cargo check` or `cargo generate-lockfile` after changing
`Cargo.toml` version, then commit the updated `Cargo.lock`.

### `lock file version 4 was found` (Debian/Fedora)

**Cause**: System Rust is too old (< 1.87). Cargo.lock v4 requires Rust 1.87+.

**Fix**: Use rustup to install the latest stable Rust (see Mirror Configuration
section above).

### SSL errors when cloning or downloading (Debian/Fedora)

**Cause**: Container SSL certificate issues under rootless Podman.

**Fix**: Use `GIT_SSL_NO_VERIFY=1` for git clone, and `curl -k` for downloads.
The Dockerfiles already handle this.
