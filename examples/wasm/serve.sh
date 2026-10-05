#!/bin/sh
# Build an example for wasm and serve it on the LAN, to test touch on a phone:
# open http://<this-machine-ip>:8080 on the device.
#
#   examples/wasm/serve.sh [example]      (default: frames)
#
# Env: PROFILE (default `web`, a size-optimised profile; `dev` is huge), PORT.
# Needs: rustup target add wasm32-unknown-unknown, and a wasm-bindgen CLI that
# matches the wasm-bindgen version in Cargo.lock.
set -e
cd "$(dirname "$0")/../.."
example="${1:-frames}"
profile="${PROFILE:-web}"
dir="$profile"; [ "$profile" != dev ] || dir=debug

want=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen CLI is '${have:-missing}' but Cargo.lock needs $want. Run:" >&2
  echo "  cargo install wasm-bindgen-cli --version $want --locked" >&2
  exit 1
fi

# Only the examples that need it get the editor_cam feature.
features=""; [ "$example" = builtin ] || features="--features editor_cam"
cargo build --example "$example" $features --target wasm32-unknown-unknown --profile "$profile"
wasm-bindgen --target web --no-typescript --out-dir examples/wasm/out --out-name app \
  "target/wasm32-unknown-unknown/$dir/examples/$example.wasm"
echo "serving on http://0.0.0.0:${PORT:-8080}"
cd examples/wasm && python3 -m http.server "${PORT:-8080}" --bind 0.0.0.0
