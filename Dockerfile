# Builds the relay server AND the browser build into one image, so a single
# deployed app gives players a link that works. Run from the repo root:
#   docker build -t riposte-server .
#   docker run --rm -p 8080:8080 riposte-server
# then open http://localhost:8080

# Stage 1: compile. This stage has the whole Rust toolchain and is thrown away.
FROM rust:1-slim AS build
WORKDIR /src
# The browser build compiles the game for WebAssembly (SETUP.md section 9).
RUN rustup target add wasm32-unknown-unknown
# The workspace Cargo.toml lists game/ and rewind/, so Cargo needs to see them
# even though only the server is built natively. This also brings in
# .cargo/config.toml, which the wasm build needs (--allow-undefined).
COPY . .
RUN cargo build --release -p riposte-server
# Same steps as scripts/build-web.sh, collected into /web.
RUN cargo build --release -p riposte-game --target wasm32-unknown-unknown \
    && mkdir /web \
    && cp web/index.html web/mq_js_bundle.js /web/ \
    && cp target/wasm32-unknown-unknown/release/riposte-game.wasm /web/

# Stage 2: run. Only the compiled binary and the web files are copied in, so
# the final image has no compiler and stays small.
FROM debian:bookworm-slim
COPY --from=build /src/target/release/riposte-server /usr/local/bin/riposte-server
COPY --from=build /web /app/web
# Hosts that set PORT override this; the server reads it at start-up.
# WEB_DIR makes the server also serve the game page (server/src/web.rs).
ENV PORT=8080 WEB_DIR=/app/web
EXPOSE 8080
CMD ["riposte-server"]
