# Builds the relay server into a small image. Run from the repo root:
#   docker build -t riposte-server .
#   docker run --rm -p 8080:8080 riposte-server

# Stage 1: compile. This stage has the whole Rust toolchain and is thrown away.
FROM rust:1-slim AS build
WORKDIR /src
# The workspace Cargo.toml lists game/ and rewind/, so Cargo needs to see them
# even though only the server is built.
COPY . .
RUN cargo build --release -p riposte-server

# Stage 2: run. Only the compiled binary is copied in, so the final image has
# no compiler and stays small.
FROM debian:bookworm-slim
COPY --from=build /src/target/release/riposte-server /usr/local/bin/riposte-server
# Hosts that set PORT override this; the server reads it at start-up.
ENV PORT=8080
EXPOSE 8080
CMD ["riposte-server"]
