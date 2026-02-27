# Stage 1: Build
FROM registry.fedoraproject.org/fedora:41 AS builder

RUN dnf install -y \
    alsa-lib-devel \
    pulseaudio-libs-devel \
    clang-devel \
    cmake \
    gcc-c++ \
    cargo \
    && dnf clean all

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release

# Stage 2: Runtime
FROM registry.fedoraproject.org/fedora-minimal:41

RUN microdnf install -y \
    alsa-lib \
    pulseaudio-libs \
    curl \
    && microdnf clean all

# Download whisper model
RUN curl -fSL -o /models/ggml-base.bin \
    --create-dirs \
    https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin

COPY --from=builder /build/target/release/stt-mcp /usr/local/bin/stt-mcp

ENV WHISPER_MODEL_PATH=/models/ggml-base.bin
EXPOSE 3000

CMD ["/usr/local/bin/stt-mcp"]
