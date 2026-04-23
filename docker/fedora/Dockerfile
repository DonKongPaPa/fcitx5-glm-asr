FROM docker.io/fedora:latest

RUN rm -f /etc/yum.repos.d/fedora-cisco-openh264.repo

RUN dnf install -y \
        gcc \
        gcc-c++ \
        make \
        cmake \
        extra-cmake-modules \
        pkgconf-pkg-config \
        wayland-protocols-devel \
        git \
        ca-certificates \
        curl \
        fcitx5-devel \
        alsa-lib-devel \
        wayland-devel \
        libxkbcommon-devel \
        pipewire-devel \
        vulkan-loader-devel && \
    dnf clean all

ENV RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
ENV RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup

RUN curl -k -L -o /tmp/rustup-init \
        https://mirrors.ustc.edu.cn/rust-static/rustup/dist/x86_64-unknown-linux-gnu/rustup-init && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y && \
    rm /tmp/rustup-init
ENV PATH="/root/.cargo/bin:${PATH}"

RUN useradd -m builder && \
    cp -r /root/.cargo /home/builder/.cargo && \
    cp -r /root/.rustup /home/builder/.rustup && \
    chown -R builder:builder /home/builder/.cargo /home/builder/.rustup
ENV PATH="/home/builder/.cargo/bin:${PATH}"

USER builder
WORKDIR /home/builder

CMD ["bash"]
