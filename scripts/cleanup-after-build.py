#!/usr/bin/env python3
"""Keep runnable Linux Zeron binaries; discard build caches and Rust toolchains."""

import argparse
import contextlib
import fcntl
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


class CleanupError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise CleanupError(message)


def no_symlink_ancestors(path, root):
    require(path.is_relative_to(root), f"Path is outside cleanup root: {path}")
    candidate = root
    require(not candidate.is_symlink(), f"Refusing symlink directory: {candidate}")
    for part in path.relative_to(root).parts[:-1]:
        candidate /= part
        require(not candidate.is_symlink(), f"Refusing symlink directory: {candidate}")


def version(binary):
    require(binary.is_file() and not binary.is_symlink(), f"Not a regular executable: {binary}")
    with binary.open("rb") as source:
        require(source.read(4) == b"\x7fELF", f"Not a Linux ELF executable: {binary}")
    result = subprocess.run(
        [str(binary), "--version"], capture_output=True, text=True, timeout=15
    )
    require(
        result.returncode == 0 and result.stdout.strip().startswith("zeron "),
        f"Executable check failed for {binary}: {result.stderr.strip()}",
    )
    return result.stdout.strip()


def check_runtime(binary, deletions):
    result = subprocess.run(
        ["ldd", str(binary)], capture_output=True, text=True, timeout=15
    )
    # Statically linked executables need no shared libraries.
    if result.returncode:
        require(
            "not a dynamic executable" in result.stderr + result.stdout
            or "statically linked" in result.stderr + result.stdout,
            f"Could not inspect runtime libraries for {binary}: {result.stderr.strip()}",
        )
        return
    require("not found" not in result.stdout, f"Missing runtime library for {binary}")
    for match in re.finditer(r"(?:=>\s*)?(/\S+)", result.stdout):
        library = Path(match.group(1)).resolve()
        for deletion in deletions:
            require(
                library != deletion and not library.is_relative_to(deletion),
                f"A runtime library would be removed: {library}",
            )


@contextlib.contextmanager
def exclusive_locks(paths):
    descriptors = []
    try:
        for path in paths:
            descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
            descriptors.append(descriptor)
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise CleanupError(f"Cargo or another cleanup is using {path}; retry after it finishes.") from error
        yield
    finally:
        for descriptor in reversed(descriptors):
            os.close(descriptor)


def allocated_bytes(paths):
    result = subprocess.run(
        ["du", "-s", "--block-size=1", "--", *(str(path) for path in paths)],
        capture_output=True,
        text=True,
        check=True,
    )
    return sum(int(line.split(maxsplit=1)[0]) for line in result.stdout.splitlines())


def remove(path):
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)


def dev_environment(value):
    if value:
        prefix = Path(value).absolute()
    else:
        active = Path(os.environ.get("CONDA_PREFIX", ""))
        prefix = active if active.name == "zeron-dev" else Path.home() / "miniconda3/envs/zeron-dev"
    require(".." not in prefix.parts, "Use an environment path without '..' components.")
    for directory in (prefix, *prefix.parents):
        require(not directory.is_symlink(), f"Refusing symlink environment directory: {directory}")
    prefix = prefix.resolve()
    require(
        prefix.name == "zeron-dev" and prefix.parent.name == "envs",
        "--dev-prefix must identify an envs/zeron-dev Conda environment.",
    )
    require((prefix / "conda-meta/history").is_file(), f"Conda environment not found: {prefix}")
    return prefix


def check_rust_processes(prefix):
    rust_home = prefix / "share/rustup/toolchains"
    shims = {prefix / "bin" / name for name in ("cargo", "rustc", "rustup", "rustfmt", "clippy-driver")}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            executable = Path(os.readlink(entry / "exe"))
        except OSError:
            continue
        require(
            executable not in shims and not executable.is_relative_to(rust_home),
            f"Rust is still running in zeron-dev (PID {entry.name}); retry after it finishes.",
        )


def clean(args):
    require(sys.platform.startswith("linux"), "This script supports the Linux development environment.")
    repo = Path(__file__).resolve().parent.parent
    require(
        (repo / "Cargo.toml").is_file() and (repo / "apps/zeron/Cargo.toml").is_file(),
        "Run the script from its location in the Zeron repository.",
    )
    target = repo / "target"
    require(target.is_dir() and not target.is_symlink(), f"Not a regular target directory: {target}")

    binaries = []
    deletions = []
    locks = [target / ".cleanup-after-build.lock"]
    profile_locks = (".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock")
    for name in ("debug", "release"):
        profile = target / name
        require(not profile.is_symlink(), f"Refusing symlink profile: {profile}")
        if not profile.exists():
            continue
        require(profile.is_dir(), f"Not a profile directory: {profile}")
        binary = profile / "zeron"
        # Keep an existing profile's Cargo lock inode throughout cleanup.
        locks.extend(profile / lock for lock in profile_locks)
        if binary.exists():
            binaries.append(binary)
        deletions.extend(
            path for path in sorted(profile.iterdir())
            if path.name != "zeron" and path.name not in profile_locks
        )
    require(binaries, "No Zeron executable found. Complete a successful build first.")
    deletions.extend([target / "tmp", target / ".rustc_info.json"])
    roots = [target]

    prefix = None
    if not args.keep_rust:
        prefix = dev_environment(args.dev_prefix)
        rust_home = prefix / "share/rustup"
        cache_paths = [prefix / "registry", prefix / "git"]
        cache_paths.extend(rust_home / name for name in ("toolchains", "downloads", "tmp", "update-hashes"))
        cache_paths.extend(prefix / name for name in (".global-cache", ".global-cache-shm", ".global-cache-wal"))
        for path in cache_paths:
            no_symlink_ancestors(path, prefix)
        deletions.extend(cache_paths)
        # Cargo takes these locks for destructive package-cache mutations.
        # Keep their inodes so a new Cargo process waits until cleanup ends.
        locks.extend([prefix / ".package-cache-mutate", prefix / ".package-cache"])
        roots.append(prefix)

    versions = {}
    for binary in binaries:
        versions[binary] = version(binary)
        check_runtime(binary, deletions)
    if not args.keep_debug_symbols:
        require(shutil.which("strip"), "Install binutils (strip), or use --keep-debug-symbols.")
    if prefix:
        check_rust_processes(prefix)

    print("Keep runnable binaries:")
    for binary in binaries:
        print(f"  {binary} ({versions[binary]})")
    if not args.keep_debug_symbols:
        print("Remove DWARF debug symbols from the kept binaries.")
    print("Remove build caches and tools:")
    for path in deletions:
        if path.exists() or path.is_symlink():
            print(f"  {path}")
    if args.dry_run:
        print("Dry run: no files changed.")
        return

    with exclusive_locks(locks), contextlib.ExitStack() as stack:
        if prefix:
            check_rust_processes(prefix)
        before = allocated_bytes(roots)
        prepared = []
        # Prepare and validate every copy before replacing a binary or deleting
        # caches. A copy also breaks Cargo's hard links into target/deps.
        for binary in binaries:
            require(version(binary) == versions[binary], f"Binary changed during cleanup: {binary}")
            check_runtime(binary, deletions)
            if args.keep_debug_symbols:
                continue
            directory = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix=".cleanup-stage-", dir=binary.parent)))
            staged = directory / "zeron"
            shutil.copy2(binary, staged)
            subprocess.run(["strip", "--strip-debug", "--", str(staged)], check=True)
            require(version(staged) == versions[binary], f"Stripped binary validation failed: {binary}")
            check_runtime(staged, deletions)
            with staged.open("rb") as source:
                os.fsync(source.fileno())
            prepared.append((staged, binary))
        for staged, binary in prepared:
            os.replace(staged, binary)
        for path in deletions:
            remove(path)
        for binary in binaries:
            require(version(binary) == versions[binary], f"Final executable validation failed: {binary}")
        after = allocated_bytes(roots)

    print(f"Freed approximately {max(0, before - after) / 1024**3:.2f} GiB.")
    print("Launch directly with zeron; cargo run requires reinstalling Rust and rebuilding.")
    if prefix:
        print("Before the next build: activate zeron-dev, run ./scripts/bootstrap-conda-rust.sh, then cargo build -p zeron.")
    print("A running Zeron may retain its old binary on disk until restarted.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dry-run", action="store_true", help="Show the cleanup plan without changing files.")
    parser.add_argument("--keep-rust", action="store_true", help="Keep Rust toolchains and Cargo download caches; only clean target.")
    parser.add_argument("--keep-debug-symbols", action="store_true", help="Keep the executable's debug symbols.")
    parser.add_argument("--dev-prefix", type=Path, help="Explicit envs/zeron-dev path instead of the active/default environment.")
    args = parser.parse_args()
    try:
        clean(args)
    except (CleanupError, OSError, subprocess.SubprocessError) as error:
        print(f"Cleanup stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
