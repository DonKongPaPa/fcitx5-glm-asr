FROM docker.io/debian:trixie

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        cmake \
        extra-cmake-modules \
        pkg-config \
        wayland-protocols \
        git \
        ca-certificates \
        curl \
        libfcitx5core-dev \
        libfcitx5config-dev \
        libfcitx5utils-dev \
        libasound2-dev \
        libwayland-dev \
        libxkbcommon-dev \
        libpipewire-0.3-dev \
        libvulkan-dev && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

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
