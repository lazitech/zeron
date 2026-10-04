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
  libvulkan-dev libwebkit2gtk-4.1-dev libjson-glib-dev libssl-dev
```

The Conda environment supplies `cmake` and `ninja`. Cargo's Linux browser
helper uses Ubuntu's `/usr/bin/pkg-config`; Conda's pkg-config can rewrite
system include paths and prevent it from finding the installed GLib headers.
The C/C++ compiler and desktop libraries above come from Ubuntu so the built
application uses the same native library ABI as the running desktop.
The setup script adds Ubuntu's multiarch pkg-config directories to this Conda
environment so Cargo build scripts can discover those system libraries.

The desktop dictation dependency also needs OpenSSL development files. If
administrator access is unavailable, extract Ubuntu's `libssl-dev` package
inside the Conda prefix and link its library names to Ubuntu's installed
OpenSSL runtime. Use development headers from the same OpenSSL version as
the installed `libssl3t64` package:

```sh
mkdir -p "$CONDA_PREFIX/share/ubuntu-openssl"
cd "$CONDA_PREFIX/share/ubuntu-openssl"
apt download libssl-dev
dpkg-deb -x libssl-dev_*.deb .
triplet="$(gcc -dumpmachine)"
ln -s "../$triplet/openssl/opensslconf.h" usr/include/openssl/opensslconf.h
ln -s "../$triplet/openssl/configuration.h" usr/include/openssl/configuration.h
ln -s "/usr/lib/$triplet/libssl.so.3" "usr/lib/$triplet/libssl.so.3"
ln -s "/usr/lib/$triplet/libcrypto.so.3" "usr/lib/$triplet/libcrypto.so.3"
conda env config vars set --name zeron-dev \
  "OPENSSL_INCLUDE_DIR=$CONDA_PREFIX/share/ubuntu-openssl/usr/include" \
  "OPENSSL_LIB_DIR=$CONDA_PREFIX/share/ubuntu-openssl/usr/lib/$triplet" \
  OPENSSL_STATIC=0
conda deactivate
conda activate zeron-dev
```

Return to the repository before building. This keeps the resulting application
on Ubuntu's OpenSSL runtime libraries.

After creating the environment and installing the system packages, work from
the repository with the environment active. For example:

```sh
cargo fmt --all
cargo run -p zeron
```
