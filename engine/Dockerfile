# Metrics' engine for external sources, as a container. Built from this repository's root:
#   docker build -f engine/Dockerfile -t ferrum-metrics-engine .
# It runs as an unprivileged user and needs FERRUM_METRICS_ENGINE_CONFIG and FERRUM_METRICS_ENGINE_TOKEN
# (docs/engine.md); its datasets are mounted read-only where its configuration says.
FROM rust:1.95-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1 AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p ferrum-metrics-engine

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
RUN useradd --system --uid 10001 --no-create-home engine
COPY --from=build /src/target/release/ferrum-metrics-engine /usr/local/bin/ferrum-metrics-engine
USER 10001
ENV FERRUM_METRICS_ENGINE_ADDR=0.0.0.0:5180
EXPOSE 5180
HEALTHCHECK --interval=30s --timeout=5s CMD ["/bin/bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/5180 && printf 'GET /health HTTP/1.0\\r\\n\\r\\n' >&3 && grep -q '\"ok\":true' <&3"]
ENTRYPOINT ["/usr/local/bin/ferrum-metrics-engine"]
