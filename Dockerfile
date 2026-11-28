# lkj-engine: Render pe build aur deploy (M4 + M5.0.1)
#
# 4 stage:
#   1. ffmpeg  : chhota custom FFmpeg (sirf libx264 + mp4) source se
#   2. rust    : engine ki release binary
#   3. assets  : TEST ke dummy images (examples/generate_assets.py se)
#   4. runtime : sirf binary + ffmpeg + assets, kuch aur nahi
#
# Rust aur runtime dono Debian bookworm hain, to glibc ka mismatch nahi hoga.
# Secrets image mein nahi jate. Render ke Environment mein ye 3 daalne hain:
#   ENGINE_KEY, TELEGRAM_BOT_TOKEN, TELEGRAM_CHAT_ID
# (PORT Render khud deta hai.)

# ---------- 1. FFmpeg (libx264 + MP4 only) ----------
FROM debian:bookworm-slim AS ffmpeg
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config nasm git curl xz-utils ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# x264: sirf static library, 8-bit.
RUN git clone --depth 1 --branch stable https://code.videolan.org/videolan/x264.git /tmp/x264 \
    && cd /tmp/x264 \
    && ./configure --prefix=/opt/x264 --enable-static --disable-cli --disable-opencl --bit-depth=8 \
    && make -j"$(nproc)" \
    && make install

# FFmpeg: sab kuch band, sirf zaroori cheezein chalu.
RUN curl -fsSL https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz -o /tmp/ffmpeg.tar.xz \
    && mkdir /tmp/ffmpeg-src \
    && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp/ffmpeg-src --strip-components=1 \
    && cd /tmp/ffmpeg-src \
    && PKG_CONFIG_PATH=/opt/x264/lib/pkgconfig ./configure \
        --prefix=/opt/ffmpeg \
        --pkg-config-flags="--static" \
        --disable-everything --disable-autodetect --disable-network \
        --disable-doc --disable-debug --disable-ffplay --disable-ffprobe \
        --enable-small --enable-gpl --enable-libx264 \
        --enable-encoder=libx264 \
        --enable-decoder=rawvideo --enable-demuxer=rawvideo \
        --enable-muxer=mp4 \
        --enable-parser=h264 \
        --enable-protocol=pipe --enable-protocol=file \
        --enable-filter=scale --enable-filter=format --enable-filter=null \
        --enable-filter=buffer --enable-filter=buffersink \
    && make -j"$(nproc)" \
    && make install

# Build ke andar hi jaanch: rgba frames pipe se lo, MP4 banao. Fail hua to build yahin ruk jayega.
RUN head -c $((64*64*4*5)) /dev/zero \
    | /opt/ffmpeg/bin/ffmpeg -hide_banner -loglevel error -y \
        -f rawvideo -pix_fmt rgba -s 64x64 -r 15 -i pipe:0 \
        -an -c:v libx264 -preset ultrafast -crf 28 -pix_fmt yuv420p \
        -movflags +faststart /tmp/smoke.mp4 \
    && test -s /tmp/smoke.mp4 \
    && ls -l /opt/ffmpeg/bin/ffmpeg /tmp/smoke.mp4

# ---------- 2. Engine ----------
FROM rust:1-bookworm AS rust
WORKDIR /src
COPY Cargo.* ./
COPY src ./src
RUN cargo build --release \
    && ls -l target/release/lkj-engine

# ---------- 3. TEST assets (dummy parts) ----------
# Asli images baad mein bahar se feed hongi. Abhi sirf test job chalane ke liye.
FROM debian:bookworm-slim AS assets
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-pil \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY examples ./examples
RUN python3 examples/generate_assets.py

# ---------- 4. Runtime ----------
FROM debian:bookworm-slim AS runtime
RUN useradd --create-home --shell /usr/sbin/nologin app
COPY --from=ffmpeg /opt/ffmpeg/bin/ffmpeg /usr/local/bin/ffmpeg
COPY --from=rust /src/target/release/lkj-engine /usr/local/bin/lkj-engine
COPY --from=assets /src/examples /app/assets
USER app
WORKDIR /app
# Render PORT env variable deta hai; engine wahi padhta hai.
CMD ["lkj-engine", "--serve", "/app/assets", "/tmp/lkj_jobs"]
