# Release Process

## Versioning

- **Semantic Versioning**: `MAJOR.MINOR.PATCH` (e.g., `0.1.2`)
- **PKGBUILD `pkgrel`**: Bumped when the source code doesn't change but the
  build does (e.g., dependency fix, PKGBUILD fix). Reset to `1` on new
  `pkgver`.

## Release Checklist

### 1. Update Version Numbers

Edit in three files:

| File | Field |
|------|-------|
| `daemon/Cargo.toml` | `version = "x.y.z"` |
| `CMakeLists.txt` | project version |
| `PKGBUILD` | `pkgver=x.y.z`, `pkgrel=1` |

### 2. Regenerate Cargo.lock

```bash
cd daemon && cargo check
# Cargo.lock is updated with the new version
```

Verify: `grep -A2 'name = "glm-asrd"' Cargo.lock` should show the new version.

### 3. Docker Verification

All three platforms must pass:

```bash
cd docker/arch    && podman compose up
cd docker/debian  && podman compose up
cd docker/fedora  && podman compose up
```

### 4. Commit and Tag

```bash
git add -A
git commit -m "chore: bump version to x.y.z"
git tag -f vx.y.z
```

### 5. Push

```bash
git push
git push -f --tags
```

### 6. CI Auto-Publish (v0.2.0+)

Pushing a `v*` tag triggers `.github/workflows/release.yml`, which:

1. Builds on 3 platforms in parallel (Arch, Debian, Fedora containers)
2. Packages artifacts:
   - `.pkg.tar.zst` (Arch)
   - `.deb` (Debian/Ubuntu)
   - `.rpm` (Fedora/RHEL)
   - `.tar.gz` (universal tarball + `install.sh`)
3. Creates a GitHub Release with all artifacts attached

## Artifacts (v0.2.0+)

| File | Platform | Install |
|------|----------|---------|
| `fcitx5-glm-asr-x.y.z-1-x86_64.pkg.tar.zst` | Arch Linux | `pacman -U <file>` |
| `fcitx5-glm-asr_x.y.z_amd64.deb` | Debian/Ubuntu | `dpkg -i <file>` |
| `fcitx5-glm-asr-x.y.z-1.fc41.x86_64.rpm` | Fedora | `dnf install <file>` |
| `fcitx5-glm-asr-x.y.z-linux-x86_64.tar.gz` | Any Linux | `tar xzf <file> && sudo ./install.sh` |

## Hotfix (pkgrel bump)

For build fixes without source changes:

```bash
# Only edit PKGBUILD: pkgrel=1 → pkgrel=2
vim PKGBUILD

# Docker verify (Arch only needed for PKGBUILD changes)
cd docker/arch && podman compose up

# Commit, re-tag, force push
git add PKGBUILD
git commit -m "fix: <description>"
git tag -f vx.y.z
git push && git push -f --tags
```

Do NOT create a new tag for pkgrel bumps — just force-update the existing tag.

## CI Configuration

Located at `.github/workflows/release.yml`.

- **Trigger**: `push` on tags matching `v*`
- **Jobs**: `build-arch`, `build-debian`, `build-fedora` (parallel)
- **Final job**: `release` — downloads all artifacts, creates GitHub Release
- **Packaging tool**: `fpm` (Effing Package Management) for `.deb` and `.rpm`
