FROM rust:slim AS chef

RUN update-ca-certificates

RUN cargo install cargo-chef
WORKDIR /zia

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ARG COMPONENT

ENV USER=zia
ENV UID=10001

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"

COPY --from=planner /zia/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN cargo build --release --package zia-${COMPONENT}

FROM gcr.io/distroless/cc
ARG COMPONENT

ENV LISTEN_ADDR=0.0.0.0:3000
EXPOSE 3000

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /zia

COPY --from=builder /zia/target/release/zia-${COMPONENT} ./zia

USER zia:zia

ENTRYPOINT ["/zia/zia"]
