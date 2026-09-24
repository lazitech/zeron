# Zeron development environment (Linux)

The repository's Conda environment provides the Cargo build helpers. The Rust
toolchain follows `rust-toolchain.toml`; `scripts/setup-conda-dev.sh` installs
rustup, rustfmt, and Clippy under the `zeron-dev` environment prefix.

## Create the environment

```sh
./scripts/setup-conda-dev.sh
source "$HOME/miniconda3/etc/profile.d/conda.sh"
conda activate zeron-dev
```

If Conda is installed elsewhere, source its `<base>/etc/profile.d/conda.sh`.
If Conda is not on `PATH`, set `CONDA_EXE` to its executable before running the
setup script. Cargo's registry and Rust toolchains stay inside the Conda
environment.

## Linux system libraries

GPUI and the Linux browser helper need Ubuntu development headers and runtime
libraries in addition to the Conda environment. Install the system compiler and
the native packages used by the repository's Linux CI workflow:

```sh
sudo apt-get install \
  build-essential \
  pkg-config \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-dev libxcb1-dev libx11-xcb-dev \
  libfontconfig1-dev libfreetype-dev libasound2-dev \
  libvulkan-dev libwebkit2gtk-4.1-dev libjson-glib-dev
```

The Conda environment supplies `cmake` and `ninja`. Cargo's Linux browser
helper uses Ubuntu's `/usr/bin/pkg-config`; Conda's pkg-config can rewrite
system include paths and prevent it from finding the installed GLib headers.
The C/C++ compiler and desktop libraries above come from Ubuntu so the built
application uses the same native library ABI as the running desktop.
The setup script adds Ubuntu's multiarch pkg-config directories to this Conda
environment so Cargo build scripts can discover those system libraries.

After creating the environment and installing the system packages, work from
the repository with the environment active. For example:

```sh
cargo fmt --all
cargo run -p zeron
```
