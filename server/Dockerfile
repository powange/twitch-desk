# Serveur de synchronisation de Twitch Desk
# cargo-chef : les dépendances sont compilées dans une couche à part, gardée en cache tant
# qu'elles ne changent pas (un changement de version du serveur ne les recompile pas)
# Images de base à version fixe (mises à jour par Dependabot : .github/dependabot.yml)
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.99.1-bookworm AS chef
WORKDIR /src

FROM chef AS plan
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS build
COPY --from=plan /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-20261027-slim
RUN useradd --system --uid 10001 tdsync && mkdir /data && chown tdsync /data
COPY --from=build /src/target/release/twitch-desk-sync /usr/local/bin/twitch-desk-sync
USER tdsync
ENV SYNC_DB=/data/sync.db SYNC_PORT=8787
VOLUME /data
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s CMD ["twitch-desk-sync", "health"]
CMD ["twitch-desk-sync"]
