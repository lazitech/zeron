#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${CONDA_PREFIX:-}" ]]; then
  echo "Activate the zeron-dev Conda environment first." >&2
  exit 1
fi

# Keep rustup, its toolchains, Cargo's registry, and installed components inside
# the Conda environment instead of writing to ~/.cargo or ~/.rustup.
export CARGO_HOME="$CONDA_PREFIX"
export RUSTUP_HOME="$CONDA_PREFIX/share/rustup"

case "$(uname -m)" in
  x86_64) rust_target=x86_64-unknown-linux-gnu ;;
  aarch64) rust_target=aarch64-unknown-linux-gnu ;;
  *) echo "Unsupported Rust bootstrap architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [[ ! -x "$CARGO_HOME/bin/rustup" ]]; then
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT
  curl --proto '=https' --tlsv1.2 -fsSL \
    "https://static.rust-lang.org/rustup/dist/${rust_target}/rustup-init" \
    -o "$tmp_dir/rustup-init"
  chmod +x "$tmp_dir/rustup-init"
  "$tmp_dir/rustup-init" -y --no-modify-path --profile minimal --default-toolchain stable
fi

export PATH="$CARGO_HOME/bin:$PATH"
rustup toolchain install stable --component rustfmt --component clippy

echo "Rust toolchain installed in $CONDA_PREFIX"
rustc --version
cargo --version
