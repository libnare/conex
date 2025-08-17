FROM --platform=$BUILDPLATFORM rust:1.89.0-alpine AS builder
WORKDIR /app

ARG APP_NAME=conex
ARG PROFILE=release

RUN apk add --no-cache musl-dev

RUN USER=root cargo new --bin ${APP_NAME}
WORKDIR /app/${APP_NAME}

COPY Cargo.toml Cargo.lock ./

ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
      "linux/amd64") export RUST_TARGET="x86_64-unknown-linux-musl" ;; \
      "linux/arm64") export RUST_TARGET="aarch64-unknown-linux-musl" ;; \
      *) echo "Unsupported platform: $TARGETPLATFORM" && exit 1 ;; \
    esac && \
    rustup target add $RUST_TARGET

ARG TARGETPLATFORM
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/${APP_NAME}/target \
    case "$TARGETPLATFORM" in \
      "linux/amd64") export RUST_TARGET="x86_64-unknown-linux-musl" ;; \
      "linux/arm64") export RUST_TARGET="aarch64-unknown-linux-musl" ;; \
    esac && \
    RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --profile ${PROFILE} --target $RUST_TARGET && \
    rm -rf target/$RUST_TARGET/${PROFILE}/.fingerprint/${APP_NAME}-*

COPY src ./src/

ARG TARGETPLATFORM
RUN --mount=type=cache,target=/app/${APP_NAME}/target \
    case "$TARGETPLATFORM" in \
      "linux/amd64") export RUST_TARGET="x86_64-unknown-linux-musl" ;; \
      "linux/arm64") export RUST_TARGET="aarch64-unknown-linux-musl" ;; \
    esac && \
    RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --profile ${PROFILE} --target $RUST_TARGET && \
    mkdir -p /tmp/app && \
    cp target/$RUST_TARGET/${PROFILE}/${APP_NAME} /tmp/app/${APP_NAME} && \
    strip /tmp/app/${APP_NAME}

FROM scratch AS runtime

COPY --from=builder /tmp/app/conex /conex

EXPOSE 8080

ENTRYPOINT ["/conex"]