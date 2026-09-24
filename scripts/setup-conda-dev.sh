#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
conda_exe="${CONDA_EXE:-$(command -v conda || true)}"
if [[ -z "$conda_exe" && -x "$HOME/miniconda3/bin/conda" ]]; then
  conda_exe="$HOME/miniconda3/bin/conda"
fi
if [[ -z "$conda_exe" ]]; then
  echo "Conda was not found. Install Miniforge/Miniconda or set CONDA_EXE." >&2
  exit 1
fi

"$conda_exe" env create --file "$repo_root/environment.yml"
prefix="$("$conda_exe" run --no-capture-output --name zeron-dev bash -c 'printf %s "$CONDA_PREFIX"')"
host_triplet="$(gcc -dumpmachine 2>/dev/null || true)"
if [[ -z "$host_triplet" ]]; then
  case "$(uname -m)" in
    x86_64) host_triplet=x86_64-linux-gnu ;;
    aarch64) host_triplet=aarch64-linux-gnu ;;
    *) echo "Cannot determine the Linux multiarch library path." >&2; exit 1 ;;
  esac
fi
pkg_config_path="/usr/lib/${host_triplet}/pkgconfig:/usr/lib/pkgconfig:/usr/share/pkgconfig:/usr/local/lib/${host_triplet}/pkgconfig:/usr/local/lib/pkgconfig:/usr/local/share/pkgconfig"

# Conda restores these values automatically when the environment is deactivated.
"$conda_exe" env config vars set --prefix "$prefix" \
  "CARGO_HOME=$prefix" \
  "RUSTUP_HOME=$prefix/share/rustup" \
  "PKG_CONFIG_PATH=$pkg_config_path" \
  "PKG_CONFIG=/usr/bin/pkg-config"

# Make rustup use the just-created environment for its initial installation.
CONDA_PREFIX="$prefix" "$repo_root/scripts/bootstrap-conda-rust.sh"

cat <<EOF

Environment ready: $prefix
Activate it with:
  source "$($conda_exe info --base)/etc/profile.d/conda.sh"
  conda activate zeron-dev

Linux system development packages are documented in docs/development/conda.md.
EOF
