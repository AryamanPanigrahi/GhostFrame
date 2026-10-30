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

For now the workspace has only `game`. Ram adds `rewind` (R1) and `server` (S1) to `members` in the root `Cargo.toml` when creating those crates.

Nothing to install for this step. Cargo downloads crates (like macroquad) from crates.io automatically on the first build. `Cargo.lock` is committed, so everyone builds with the same versions.

## 4. Build, test, run

```bash
cargo build --workspace
```

```bash
cargo test --workspace
```

```bash
cargo run -p riposte-game
```

The first build downloads and compiles macroquad and takes a minute; later builds are incremental.

Before committing, check formatting and lints:

```bash
cargo fmt --all --check
```

```bash
cargo clippy --workspace -- -D warnings
```
