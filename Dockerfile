FROM rust:1.98-alpine AS build

ARG VERSION
WORKDIR /build
RUN apk add --no-cache bash
COPY Cargo.toml Cargo.lock ./
COPY apps/media-projector/Cargo.toml apps/media-projector/Cargo.toml
COPY apps/media-projector/build.rs apps/media-projector/build.rs
COPY apps/media-projector/src apps/media-projector/src
COPY scripts/version.sh scripts/version.sh
RUN test -n "$VERSION"
RUN VERSION="$VERSION" cargo build --locked --release

FROM scratch

COPY --from=build /build/target/release/media-projector /media-projector
ENTRYPOINT ["/media-projector"]
CMD ["--source", "/storage/torrents/media", "--target", "/storage/jellyfin"]
