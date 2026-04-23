FROM docker.io/fedora:41

RUN dnf install -y \
        @development-tools \
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

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

RUN useradd -m builder && \
    cp -r /root/.cargo /home/builder/.cargo && \
    cp -r /root/.rustup /home/builder/.rustup && \
    chown -R builder:builder /home/builder/.cargo /home/builder/.rustup
ENV PATH="/home/builder/.cargo/bin:${PATH}"

USER builder
WORKDIR /home/builder

CMD ["bash"]
