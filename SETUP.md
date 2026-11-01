# SETUP.md

Every setup step for Riposte, in order, with what it is and why it's needed. Run these by hand; nothing here is meant to happen invisibly.

---

## 1. Rust toolchain

**What:** `rustup` installs and updates the Rust compiler (`rustc`), the build tool and package manager (`cargo`), and extra components.
**Why:** The whole project is Rust. We use the stable channel, edition 2021.

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On Windows, download `rustup-init.exe` from https://rustup.rs instead.

On Windows the default toolchain (`msvc`) links with Microsoft's C++ Build Tools, a separate multi-GB Visual Studio install. If you don't have them, use the `gnu` toolchain instead: it ships its own linker, so nothing else is needed. This is what Ram's machine uses:

```bash
rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal -c rustfmt -c clippy
```

Then open a new terminal so `%USERPROFILE%\.cargo\bin` is on `PATH`.

Then add the formatter and linter. CLAUDE.md requires code to be `cargo fmt` clean and `cargo clippy` clean:

```bash
rustup component add rustfmt clippy
```

Check that it worked:

```bash
cargo --version
```

## 2. System libraries for the game window (Linux only)

**What:** macroquad, our window and drawing library, opens the window through X11 and draws with OpenGL. On Linux, it loads these system libraries when the game starts.
**Why:** If they're missing, `cargo run -p riposte-game` builds but fails to open a window. macOS and Windows already ship what's needed, so skip this step there.

Debian or Ubuntu:

```bash
sudo apt install libx11-dev libxi-dev libgl1-mesa-dev
```

Fedora:

```bash
sudo dnf install libX11-devel libXi-devel mesa-libGL-devel
```

We don't turn on macroquad's `audio` feature, so ALSA (`libasound2-dev`) isn't needed.

## 3. The Cargo workspace

**What:** The root `Cargo.toml` declares a *workspace*, a group of crates that share one `Cargo.lock` (exact dependency versions) and one `target/` build folder.
**Why:** `rewind`, `riposte-game` and `riposte-server` are built and tested together with one command. Each crate still has its own `Cargo.toml` listing its own dependencies.

The workspace has `game`, `rewind` (added in R1) and `server` (added in S1).

Nothing to install for this step. Cargo downloads crates (like macroquad) from crates.io automatically on the first build. `Cargo.lock` is committed, so everyone builds with the same versions.

## 4. rewind's first dependency: serde

**What:** `serde` is Rust's standard serialization framework. It defines two traits, `Serialize` (turn a value into bytes or text) and `Deserialize` (turn them back), that formats like `bincode` plug into.
**Why:** In rewind's `RollbackGame` trait, a game's `Input` type must be `Serialize + DeserializeOwned`, because inputs are the one thing peers send each other. R1 only needs the traits, so the `derive` feature is not turned on yet.

It's declared in `rewind/Cargo.toml`:

```toml
serde = "1"
```

Nothing to run: Cargo downloads it on the next build and records the exact version in `Cargo.lock`.

**R3 adds two things to `rewind/Cargo.toml`:**
- serde's `derive` feature, which generates the serialization code for the `PeerMsg` enum.
- `bincode` (version 1, the same as the relay server), which turns a `PeerMsg` into compact bytes and back.

Both are listed for rewind in DESIGN.md section 9. `bincode` was already in `Cargo.lock` through the server, so nothing new is downloaded:

```toml
serde = { version = "1", features = ["derive"] }
bincode = "1"
```

## 5. Build, test, run

```bash
cargo build --workspace
```

```bash
cargo test --workspace
```

```bash
cargo run -p riposte-game
```

Test only the rollback library:

```bash
cargo test -p rewind
```

Run Riposte itself under the sync test (G5): 4 seeds × 6000 frames of bot-vs-bot, rolling back 8 frames every tick. `--nocapture` prints how many hits, KOs and matches each run covered:

```bash
cargo test -p riposte-game sync -- --nocapture
```

The game uses `rewind` as a *path dependency* (`rewind = { path = "../rewind" }` in `game/Cargo.toml`): Cargo builds it from the folder next door instead of downloading it, so there's nothing to install.

Run the network convergence tests (R4): 200 random bad networks (latency, jitter, loss, reordering), and both peers must end identical. `--nocapture` prints how much rolling back and message loss happened:

```bash
cargo test -p rewind --test convergence -- --nocapture
```

Run rewind's toy game under the sync test (and the deliberately broken version, which should fail with a desync):

```bash
cargo run -p rewind --example boxes
```

```bash
cargo run -p rewind --example boxes -- --broken
```

The first build downloads and compiles macroquad and takes a minute; later builds are incremental.

**Practice vs lag** (main menu, second item): you against the bot through real rollback netcode over the fake network. You can use either key set. Number keys **1-4** switch the lag mid-match: none, 33 ms, 67 ms, then 100 ms one-way with jitter and some lost messages. The overlay shows how many frames have been rolled back. Esc returns to the menu.

Controls for local 2-player:

| | Move | Jump | Crouch | Light | Heavy |
|---|---|---|---|---|---|
| P1 | A / D | W | S | F | G |
| P2 | ← / → | ↑ | ↓ | K | L |

Before committing, check formatting and lints:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace -- -D warnings
```

## 6. The relay server's dependencies (S1)

All of these are declared in `server/Cargo.toml`. Nothing to install by hand: Cargo downloads them on the next build and pins exact versions in `Cargo.lock`.

| Crate | What it is | Why we need it |
|---|---|---|
| `tokio` | Rust's async runtime: it runs many tasks (one per connected player) on a few threads, and waits for network I/O without blocking a thread each. | A relay spends nearly all its time waiting for bytes, so async fits. We turn on only the features we use (`net`, `time`, `sync`, `signal`, `macros`, `rt-multi-thread`). |
| `axum` (feature `ws`) | A small web framework on top of tokio. The `ws` feature adds WebSocket upgrades. | Gives us the `/ws` route and the WebSocket type without writing the HTTP handshake ourselves. |
| `serde` (feature `derive`) + `bincode` | `serde` is the serialization framework from section 4; `derive` generates the code for our message enums. `bincode` turns a value into compact bytes and back. | `ClientMsg` and `ServerMsg` travel as bincode in binary WebSocket frames (DESIGN.md section 6). |
| `tracing` + `tracing-subscriber` | Structured logging. The subscriber prints the logs; `env-filter` lets you pick the level with the `RUST_LOG` environment variable. | Logs for room created, match started, protocol errors, disconnects (S2). |
| `anyhow` | Easy error handling for programs (used in `main.rs` only). | CLAUDE.md: `anyhow` in binaries. |

Two extra crates are **dev-dependencies**, which means they are only compiled for `cargo test`, never into the server:

- `tokio-tungstenite`: a WebSocket *client*. `server/tests/relay.rs` uses it to connect to a real running server.
- `futures-util`: the `.send()` and `.next()` helpers for that client.

Neither is in DESIGN.md section 9, so they are justified here: the tests talk to the server over a real socket, which is the only way to check the wire format and the close behaviour. Both are already pulled in by `axum`, so they add no new code to the dependency tree.

## 7. Run the relay server (S1, S2)

```bash
cargo run -p riposte-server
```

It listens on `0.0.0.0:8080`, WebSocket route `/ws`. Set `PORT` to change the port, and `RUST_LOG=debug` for more logging. In PowerShell:

```powershell
$env:PORT = "9000"; $env:RUST_LOG = "debug"; cargo run -p riposte-server
```

Open http://localhost:8080/ in a browser: it answers `riposte-server ok`, which is also what a hosting platform's health check looks for. Stop it with Ctrl-C.

## 8. Docker image for the server (S2)

**What:** Docker packages a program and everything it needs into an *image* that runs the same on any machine. Install Docker Desktop from https://www.docker.com/products/docker-desktop/ first.
**Why:** DESIGN.md section 8: the relay should deploy as one binary or one image to a free-tier host.

The `Dockerfile` in the repo root has two stages: the first compiles the server with the full Rust toolchain, the second copies only the finished binary into a small Debian image. The first build downloads a few hundred MB of base images.

```bash
docker build -t riposte-server .
```

```bash
docker run --rm -p 8080:8080 riposte-server
```

The `.dockerignore` file keeps `target/` and `.git/` out of the build so the build context stays small.

## 9. Browser (WASM) build

**What:** WebAssembly (WASM) is a compact binary format that browsers run at near-native speed. Rust can compile to it through the `wasm32-unknown-unknown` *target*: "wasm32" is the CPU, and "unknown-unknown" means no particular vendor and no operating system, because the browser provides everything.

**Why:** Milestone G7 / I2: Riposte should be playable from a link, with nothing to install.

**Step 1: add the target (once).** This downloads the Rust standard library precompiled for WASM:

```bash
rustup target add wasm32-unknown-unknown
```

**Step 2: build.** The script compiles the game for WASM in release mode and copies the result into `web/`:

```bash
./scripts/build-web.sh
```

It runs these two commands, which you can also run by hand (e.g. on Windows without bash):

```bash
cargo build -p riposte-game --release --target wasm32-unknown-unknown
```

```bash
cp target/wasm32-unknown-unknown/release/riposte-game.wasm web/
```

**Step 3: serve `web/` and open it.** Browsers refuse to `fetch` a `.wasm` file from a `file://` page, so double-clicking `index.html` won't work; it has to come from a web server. Python's built-in one is enough:

```bash
python3 -m http.server 8000 --directory web
```

Then open http://localhost:8000 and click the page once so it receives keyboard input.

**What's in `web/`:**
- `index.html`: a full-page canvas with the id `glcanvas`, which macroquad draws into, plus two script tags.
- `mq_js_bundle.js`: macroquad's JavaScript glue. It gives the WASM module WebGL, keyboard events and a clock. It's copied unchanged from macroquad 0.4.16's `js/` folder (MIT/Apache, like macroquad itself) and **must match the macroquad version in `game/Cargo.toml`**. If macroquad is upgraded, copy the new one from `~/.cargo/registry/src/*/macroquad-<version>/js/mq_js_bundle.js`.
- `riposte-game.wasm`: the build output. It's in `.gitignore`, so it's rebuilt rather than committed.
