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
