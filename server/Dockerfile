# Serveur de synchronisation de Twitch Desk
FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 tdsync && mkdir /data && chown tdsync /data
COPY --from=build /src/target/release/twitch-desk-sync /usr/local/bin/twitch-desk-sync
USER tdsync
ENV SYNC_DB=/data/sync.db SYNC_PORT=8787
VOLUME /data
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s CMD ["twitch-desk-sync", "health"]
CMD ["twitch-desk-sync"]
