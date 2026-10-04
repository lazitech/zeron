# Local Development Notes

## Development environment

- This machine has a Conda environment named `zeron-dev` at
  `/home/lazic/miniconda3/envs/zeron-dev`.
- From the repository root, activate it before running Rust commands:

  ```sh
  source /home/lazic/miniconda3/etc/profile.d/conda.sh
  conda activate zeron-dev
  ```

- Launch the desktop app with `cargo run -p zeron`.
- To recreate the environment on this Linux machine, run
  `./scripts/setup-conda-dev.sh`. It uses `environment.yml` and installs the
  Rust toolchain selected by `rust-toolchain.toml` inside the Conda prefix.
- Cargo's registry and Rust toolchains are kept inside the Conda environment
  (`CARGO_HOME` and `RUSTUP_HOME`). Keep this isolation; do not mix the
  environment with a separate system Cargo installation.
- Linux desktop development also needs Ubuntu system headers and libraries.
  Follow the package list and `pkg-config` notes in
  [`docs/development/conda.md`](docs/development/conda.md). In particular,
  GPUI's Linux browser helper needs `/usr/bin/pkg-config` to discover the
  system WebKit/GLib libraries; Conda's `pkg-config` can return incompatible
  paths.

## Personal usage statistics branch

- This checkout is the user's personal version. The statistics dashboard is
  intended for local use; do not open an upstream PR or push commits to the
  upstream repository unless the user explicitly asks.
- The `feature/statistics-dashboard` branch was previously pushed to the
  user's fork. New local changes, including this file, are not on that remote
  branch unless explicitly pushed.
- Statistics are assembled from local native-agent histories and local Zeron
  journals. Native histories are read-only; the SQLite index stores compact
  metadata and summaries, not transcript bodies. Statistics are not synced
  between devices.
- Heatmap color intensity represents daily interaction rounds. Hovering a day
  shows its interaction count and date-attributed Token usage in K/M/B units.
- Daily and weekly token totals need timestamped token-usage events. Some
  agents do not expose those events, so `—` means the source could not provide
  a date-specific total; it does not mean zero usage.
- Known limitation: if parsing a native file's dated token events fails, the
  error is logged, but current coverage metadata may not mark date-specific
  token statistics as incomplete. Treat those totals as potentially partial.
- Official `main` through `9e1a1115` (v0.2.102 plus subsequent updates) was
  merged on 2026-10-03 while preserving personal changes. The pre-merge state
  is saved as `backup/personal-before-upstream-20261003-f6bd1ac5`.
  Do not rebase or merge later upstream updates automatically; preserve the
  personal changes and inspect the resulting diff when an update is requested.

## Verification

- Automated builds and tests should be run when the user asks to verify them.
- Ubuntu's OpenSSL development files are extracted under
  `$CONDA_PREFIX/share/ubuntu-openssl`; environment-scoped `OPENSSL_*`
  variables select these headers and the system OpenSSL runtime libraries.
  See the administrator-free setup in `docs/development/conda.md`.
- For a requested visual check, activate `zeron-dev` and run `cargo run -p zeron`
  from the repository root.
