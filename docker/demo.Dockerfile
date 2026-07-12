# Demo-recording environment: the oops dev image plus VHS and its runtime
# dependencies (ttyd, ffmpeg, chromium). Versions are pinned so the GIF only
# changes when demo/demo.tape changes.
FROM oops-dev

ARG TARGETARCH
ARG VHS_VERSION=0.10.0
ARG TTYD_VERSION=1.7.7

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl ffmpeg chromium fonts-dejavu-core fonts-noto-color-emoji \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    case "$TARGETARCH" in \
      arm64) ttyd_arch=aarch64; vhs_arch=arm64 ;; \
      amd64) ttyd_arch=x86_64; vhs_arch=x86_64 ;; \
      *) echo "unsupported arch: $TARGETARCH" && exit 1 ;; \
    esac; \
    curl -fsSL -o /usr/local/bin/ttyd \
      "https://github.com/tsl0922/ttyd/releases/download/${TTYD_VERSION}/ttyd.${ttyd_arch}"; \
    chmod +x /usr/local/bin/ttyd; \
    curl -fsSL "https://github.com/charmbracelet/vhs/releases/download/v${VHS_VERSION}/vhs_${VHS_VERSION}_Linux_${vhs_arch}.tar.gz" \
      | tar -xz -C /tmp; \
    install -m755 "/tmp/vhs_${VHS_VERSION}_Linux_${vhs_arch}/vhs" /usr/local/bin/vhs; \
    rm -rf /tmp/vhs_*

# Headless chromium as root inside a container needs the sandbox disabled.
ENV VHS_NO_SANDBOX=true
