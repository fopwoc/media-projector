FROM rust:1.97-alpine AS build

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release

FROM scratch

COPY --from=build /build/target/release/media-projector /media-projector
ENTRYPOINT ["/media-projector"]
CMD ["--source", "/storage/torrents/media", "--target", "/storage/jellyfin"]

