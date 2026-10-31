#!/usr/bin/env bash
# Build the browser version of Riposte into web/.
# Usage (from the repo root): ./scripts/build-web.sh
# Then serve web/ with any static file server, e.g.
#   python3 -m http.server 8000 --directory web
# and open http://localhost:8000
set -euo pipefail

cargo build -p riposte-game --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/riposte-game.wasm web/
echo "Built web/riposte-game.wasm"
