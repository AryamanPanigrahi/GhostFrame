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

The workspace has `game` and `rewind` (added in R1). Ram adds `server` (S1) to `members` in the root `Cargo.toml` when creating it.

Nothing to install for this step. Cargo downloads crates (like macroquad) from crates.io automatically on the first build. `Cargo.lock` is committed, so everyone builds with the same versions.

## 4. rewind's first dependency: serde

**What:** `serde` is Rust's standard serialization framework. It defines two traits, `Serialize` (turn a value into bytes or text) and `Deserialize` (turn them back), that formats like `bincode` plug into.
**Why:** In rewind's `RollbackGame` trait, a game's `Input` type must be `Serialize + DeserializeOwned`, because inputs are the one thing peers send each other. R1 only needs the traits, so the `derive` feature is not turned on yet.

It's declared in `rewind/Cargo.toml`:

```toml
serde = "1"
```

Nothing to run: Cargo downloads it on the next build and records the exact version in `Cargo.lock`.

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

Run rewind's toy game under the sync test (and the deliberately broken version, which should fail with a desync):

```bash
cargo run -p rewind --example boxes
```

```bash
cargo run -p rewind --example boxes -- --broken
```

The first build downloads and compiles macroquad and takes a minute; later builds are incremental.

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

## 5. Browser (WASM) build

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
