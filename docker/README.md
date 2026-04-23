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
│   ├── Dockerfile         Debian bookworm (cargo + cmake)
│   └── docker-compose.yml
└── fedora/
    ├── Dockerfile         Fedora 41 (cargo + cmake)
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

- **Base image**: `debian:bookworm`
- **What it tests**: `cargo build --release --locked --features vello-renderer`
  + `cmake` + `make`
- **Key verification**: Dependency resolution on Debian/Ubuntu

### Fedora (`docker/fedora/`)

- **Base image**: `fedora:41`
- **What it tests**: Same as Debian (cargo + cmake)
- **Key verification**: Dependency resolution on Fedora/RHEL

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
