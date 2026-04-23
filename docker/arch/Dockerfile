FROM docker.io/archlinux:latest

RUN pacman -Syu --noconfirm && \
    pacman -S --noconfirm --needed \
        base-devel \
        rust \
        cargo \
        cmake \
        extra-cmake-modules \
        pkgconf \
        wayland-protocols \
        git \
        fcitx5 \
        pipewire \
        gcc-libs \
        wayland \
        vulkan-icd-loader \
        alsa-lib && \
    pacman -Sc --noconfirm

RUN useradd -m -G wheel builder && \
    sed -i 's/^# %wheel ALL=(ALL:ALL) NOPASSWD: ALL/%wheel ALL=(ALL:ALL) NOPASSWD: ALL/' /etc/sudoers

USER builder
WORKDIR /home/builder

CMD ["bash"]
