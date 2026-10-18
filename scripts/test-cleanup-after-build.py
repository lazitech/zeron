#!/usr/bin/env python3
"""Isolated regression tests for scripts/cleanup-after-build.py.

The tests copy the production script into a temporary pseudo-repository before
each case.  They never touch this checkout's target directory, Conda
environment, Cargo cache, or application data, and they deliberately invoke
the production script without Cargo.
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest


REPO_ROOT = Path(__file__).resolve().parents[1]
PRODUCTION = REPO_ROOT / "scripts" / "cleanup-after-build.py"


def _readelf_sections(path: Path) -> str:
    result = subprocess.run(
        ["readelf", "-S", str(path)],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def _assert_runs(test: unittest.TestCase, path: Path) -> None:
    result = subprocess.run(
        [str(path), "--version"],
        check=False,
        capture_output=True,
        text=True,
    )
    test.assertEqual(result.returncode, 0, result.stderr)
    test.assertEqual(result.stdout.strip(), "zeron 0.0.0")


def _snapshot_tree(path: Path) -> dict[str, tuple[object, ...]]:
    """Capture content and inode identity without depending on timestamps."""

    if not path.exists() and not path.is_symlink():
        return {}
    snapshot: dict[str, tuple[object, ...]] = {}
    for current, dirs, files in os.walk(path, followlinks=False):
        current_path = Path(current)
        for name in sorted(dirs + files):
            item = current_path / name
            relative = str(item.relative_to(path))
            item_stat = item.lstat()
            mode = stat.S_IFMT(item_stat.st_mode)
            if stat.S_ISLNK(mode):
                snapshot[relative] = ("link", os.readlink(item), item_stat.st_ino)
            elif stat.S_ISREG(mode):
                digest = hashlib.sha256(item.read_bytes()).hexdigest()
                snapshot[relative] = (
                    "file",
                    item_stat.st_ino,
                    item_stat.st_size,
                    item_stat.st_mode & 0o777,
                    digest,
                )
            else:
                snapshot[relative] = ("other", mode, item_stat.st_ino)
    return snapshot


@contextlib.contextmanager
def _held_exclusive_lock(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        fcntl.flock(fd, fcntl.LOCK_UN)
        os.close(fd)


class Fixture:
    """A temporary repository, home directory, and Conda-shaped prefix."""

    def __init__(self) -> None:
        self._temp = tempfile.TemporaryDirectory(prefix="zeron-cleanup-test-")
        self.root = Path(self._temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.scripts = self.repo / "scripts"
        self.scripts.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.target = self.repo / "target"
        self.prefix = self.repo / "envs" / "zeron-dev"
        self._install_production_script()
        self._write_pseudo_manifests()

    def close(self) -> None:
        self._temp.cleanup()

    def _install_production_script(self) -> None:
        if not PRODUCTION.is_file():
            raise unittest.SkipTest(
                f"production script is not available yet: {PRODUCTION}"
            )
        destination = self.scripts / PRODUCTION.name
        shutil.copy2(PRODUCTION, destination)
        destination.chmod(0o755)

    def _write_pseudo_manifests(self) -> None:
        (self.repo / "Cargo.toml").write_text(
            textwrap.dedent(
                """
                [workspace]
                resolver = "2"
                members = ["apps/zeron"]
                """
            ).lstrip(),
            encoding="utf-8",
        )
        app_manifest = self.repo / "apps" / "zeron" / "Cargo.toml"
        app_manifest.parent.mkdir(parents=True)
        app_manifest.write_text(
            textwrap.dedent(
                """
                [package]
                name = "zeron"
                version = "0.0.0"
                edition = "2024"
                """
            ).lstrip(),
            encoding="utf-8",
        )

    def env(self, *, path_prefix: Path | None = None) -> dict[str, str]:
        environment = os.environ.copy()
        environment["XDG_CACHE_HOME"] = str(self.home / ".cache")
        if path_prefix is not None:
            environment["PATH"] = f"{path_prefix}:/usr/bin:/bin"
        else:
            environment["PATH"] = "/usr/bin:/bin"
        environment.pop("CONDA_PREFIX", None)
        return environment

    def run(self, *arguments: str, prefix: Path | None = None, path_prefix: Path | None = None) -> subprocess.CompletedProcess[str]:
        dev_prefix = prefix or self.prefix
        return subprocess.run(
            [sys.executable, str(self.scripts / PRODUCTION.name), *arguments, "--dev-prefix", str(dev_prefix)],
            cwd=self.repo,
            env=self.env(path_prefix=path_prefix),
            check=False,
            capture_output=True,
            text=True,
        )

    def create_prefix(self, prefix: Path | None = None) -> Path:
        dev_prefix = prefix or self.prefix
        (dev_prefix / "conda-meta").mkdir(parents=True, exist_ok=True)
        (dev_prefix / "conda-meta" / "history").write_text("conda history\n", encoding="utf-8")

        # These are the Conda/Rust files the production script must preserve.
        (dev_prefix / "bin").mkdir(parents=True, exist_ok=True)
        rustup = dev_prefix / "bin" / "rustup"
        rustup.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        rustup.chmod(0o755)
        (dev_prefix / "bin" / "cargo").symlink_to("rustup")
        (dev_prefix / "bin" / "python").write_text("conda base tool\n", encoding="utf-8")
        (dev_prefix / "share" / "rustup").mkdir(parents=True, exist_ok=True)
        (dev_prefix / "share" / "rustup" / "settings.toml").write_text(
            "version = \"12\"\n", encoding="utf-8"
        )

        # Cache/toolchain payloads that should be removed.
        for relative in (
            "registry/cache/crate.tar.gz",
            "registry/index/index.json",
            "registry/src/crate/src/lib.rs",
            "git/checkouts/zui/.git/HEAD",
            "git/db/zui.git/HEAD",
            "share/rustup/toolchains/stable/bin/rustc",
            "share/rustup/downloads/rust.tar.xz",
            "share/rustup/tmp/install.tmp",
            "share/rustup/update-hashes/stable.sha256",
        ):
            file = dev_prefix / relative
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("cache\n", encoding="utf-8")
        (dev_prefix / ".global-cache").write_bytes(b"cargo global cache metadata\n")
        (dev_prefix / ".package-cache").write_bytes(b"cargo package cache\n")
        (dev_prefix / ".package-cache-mutate").write_bytes(b"cargo package cache mutate lock\n")

        user_data = self.home / ".zeron" / "user-data.json"
        user_data.parent.mkdir(parents=True, exist_ok=True)
        user_data.write_text("keep me\n", encoding="utf-8")
        return dev_prefix

    def create_target(self, *, valid_executables: bool = True, release: bool = True) -> None:
        self.target.mkdir(parents=True, exist_ok=True)
        (self.target / ".cleanup-after-build.lock").write_text(
            "cleanup lock\n", encoding="utf-8"
        )
        source = self.root / "zeron.c"
        source.write_text(
            "#include <stdio.h>\nint main(void) { puts(\"zeron 0.0.0\"); return 0; }\n",
            encoding="utf-8",
        )
        template = self.root / "zeron-template"
        if valid_executables:
            subprocess.run(
                ["gcc", "-g", "-O0", "-Wall", "-Wextra", str(source), "-o", str(template)],
                check=True,
                capture_output=True,
                text=True,
            )
        profiles = ["debug"] + (["release"] if release else [])
        for profile in profiles:
            directory = self.target / profile
            directory.mkdir(parents=True, exist_ok=True)
            if valid_executables:
                shutil.copy2(template, directory / "zeron")
                (directory / "zeron").chmod(0o755)
            else:
                bad = directory / "zeron"
                bad.write_text("#!/bin/sh\nexit 23\n", encoding="utf-8")
                bad.chmod(0o755)
            for relative in (
                "deps/old-dependency",
                "incremental/old-state",
                "build/old-build-script/output",
                ".fingerprint/old-fingerprint",
            ):
                file = directory / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("intermediate\n", encoding="utf-8")
            (directory / ".cargo-lock").write_text("target lock\n", encoding="utf-8")
            (directory / ".cargo-build-lock").write_text("target build lock\n", encoding="utf-8")
            (directory / ".cargo-artifact-lock").write_text(
                "target artifact lock\n", encoding="utf-8"
            )

        if valid_executables:
            debug_binary = self.target / "debug" / "zeron"
            self.hardlink_keeper = self.root / "debug-binary-hardlink"
            os.link(debug_binary, self.hardlink_keeper)
        else:
            self.hardlink_keeper = None

    def populate(self, *, valid_executables: bool = True, release: bool = True) -> Path:
        self.create_prefix()
        self.create_target(valid_executables=valid_executables, release=release)
        return self.prefix


class CleanupAfterBuildTests(unittest.TestCase):
    def fixture(self, **kwargs: object) -> Fixture:
        fixture = Fixture()
        self.addCleanup(fixture.close)
        fixture.populate(**kwargs)
        return fixture

    def assert_success(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(
            result.returncode,
            0,
            f"cleanup failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def assert_rejected(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertNotEqual(
            result.returncode,
            0,
            f"cleanup unexpectedly succeeded\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def assert_target_profiles_clean(self, fixture: Fixture) -> None:
        for profile in ("debug", "release"):
            directory = fixture.target / profile
            executable = directory / "zeron"
            self.assertTrue(executable.is_file(), executable)
            _assert_runs(self, executable)
            for name in ("deps", "incremental", "build", ".fingerprint"):
                self.assertFalse((directory / name).exists(), directory / name)
            for name in (".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"):
                self.assertTrue((directory / name).is_file(), directory / name)

    def assert_prefix_caches_gone(self, prefix: Path) -> None:
        for relative in (
            "registry",
            "git",
            "share/rustup/toolchains",
            "share/rustup/downloads",
            "share/rustup/tmp",
            "share/rustup/update-hashes",
            ".global-cache",
        ):
            self.assertFalse((prefix / relative).exists(), prefix / relative)

    def assert_prefix_base_preserved(self, fixture: Fixture, prefix: Path | None = None) -> None:
        dev_prefix = prefix or fixture.prefix
        self.assertEqual((dev_prefix / "conda-meta" / "history").read_text(encoding="utf-8"), "conda history\n")
        self.assertTrue((dev_prefix / "bin" / "rustup").is_file())
        self.assertTrue((dev_prefix / "bin" / "cargo").is_symlink())
        self.assertEqual((dev_prefix / "bin" / "python").read_text(encoding="utf-8"), "conda base tool\n")
        self.assertEqual(
            (dev_prefix / "share" / "rustup" / "settings.toml").read_text(encoding="utf-8"),
            "version = \"12\"\n",
        )
        self.assertEqual(
            (fixture.home / ".zeron" / "user-data.json").read_text(encoding="utf-8"),
            "keep me\n",
        )

    def test_dry_run_is_completely_read_only(self) -> None:
        fixture = self.fixture()
        before = _snapshot_tree(fixture.root)
        result = fixture.run("--dry-run")
        self.assert_success(result)
        self.assertEqual(_snapshot_tree(fixture.root), before)

    def test_default_keeps_runnable_binaries_and_cleans_caches(self) -> None:
        fixture = self.fixture()
        debug_binary = fixture.target / "debug" / "zeron"
        release_binary = fixture.target / "release" / "zeron"
        hardlink = fixture.hardlink_keeper
        assert hardlink is not None
        old_inode = hardlink.stat().st_ino
        old_bytes = hardlink.read_bytes()
        self.assertIn(".debug_info", _readelf_sections(hardlink))

        result = fixture.run()
        self.assert_success(result)
        self.assert_target_profiles_clean(fixture)
        self.assert_prefix_caches_gone(fixture.prefix)
        self.assert_prefix_base_preserved(fixture)
        self.assertNotIn(".debug_info", _readelf_sections(debug_binary))
        self.assertNotIn(".debug_info", _readelf_sections(release_binary))

        # The deps hardlink is removed with the deps directory, but its other
        # link must retain the old inode and bytes.  In-place strip would mutate
        # it and fail this assertion.
        self.assertEqual(hardlink.stat().st_ino, old_inode)
        self.assertEqual(hardlink.read_bytes(), old_bytes)
        self.assertIn(".debug_info", _readelf_sections(hardlink))
        self.assertNotEqual(debug_binary.stat().st_ino, old_inode)

    def test_keep_rust_only_cleans_target(self) -> None:
        fixture = self.fixture()
        result = fixture.run("--keep-rust")
        self.assert_success(result)
        self.assert_target_profiles_clean(fixture)
        self.assertTrue((fixture.prefix / "registry").exists())
        self.assertTrue((fixture.prefix / "git").exists())
        self.assertTrue((fixture.prefix / "share" / "rustup" / "toolchains").exists())
        self.assertTrue((fixture.prefix / ".global-cache").exists())
        self.assert_prefix_base_preserved(fixture)

    def test_keep_debug_symbols_preserves_dwarf(self) -> None:
        fixture = self.fixture()
        result = fixture.run("--keep-debug-symbols")
        self.assert_success(result)
        self.assert_target_profiles_clean(fixture)
        for profile in ("debug", "release"):
            self.assertIn(".debug_info", _readelf_sections(fixture.target / profile / "zeron"))
        self.assert_prefix_caches_gone(fixture.prefix)

    def test_missing_or_unrunnable_executable_aborts_without_deletion(self) -> None:
        for valid_executables in (False,):
            with self.subTest(valid_executables=valid_executables):
                fixture = self.fixture(valid_executables=valid_executables)
                before = _snapshot_tree(fixture.root)
                result = fixture.run()
                self.assert_rejected(result)
                self.assertEqual(_snapshot_tree(fixture.root), before)

        fixture = Fixture()
        self.addCleanup(fixture.close)
        fixture.create_prefix()
        fixture.target.mkdir(parents=True)
        (fixture.target / "debug").mkdir()
        (fixture.target / "release").mkdir()
        before = _snapshot_tree(fixture.root)
        result = fixture.run()
        self.assert_rejected(result)
        self.assertEqual(_snapshot_tree(fixture.root), before)

    def test_target_symlink_is_rejected_without_touching_target(self) -> None:
        fixture = Fixture()
        self.addCleanup(fixture.close)
        fixture.create_prefix()
        outside = fixture.root / "outside-target"
        fixture.target.parent.mkdir(parents=True, exist_ok=True)
        outside.mkdir()
        fixture.target.symlink_to(outside, target_is_directory=True)
        marker = outside / "must-survive"
        marker.write_text("outside\n", encoding="utf-8")
        before = _snapshot_tree(outside)

        result = fixture.run()
        self.assert_rejected(result)
        self.assertEqual(_snapshot_tree(outside), before)
        self.assertEqual(marker.read_text(encoding="utf-8"), "outside\n")
        self.assertTrue((fixture.prefix / "registry").exists())

    def test_prefix_ancestor_symlink_is_rejected(self) -> None:
        fixture = Fixture()
        self.addCleanup(fixture.close)
        outside_root = fixture.root / "outside-root"
        outside_envs = outside_root / "envs"
        real_prefix = outside_envs / "zeron-dev"
        fixture.create_prefix(real_prefix)
        (fixture.repo / "envs").symlink_to(outside_envs, target_is_directory=True)
        fixture.create_target()
        before = _snapshot_tree(outside_envs)

        result = fixture.run(prefix=fixture.repo / "envs" / "zeron-dev")
        self.assert_rejected(result)
        self.assertEqual(_snapshot_tree(outside_envs), before)
        self.assertTrue((real_prefix / "registry").exists())

    def test_wrong_or_incomplete_prefix_is_rejected(self) -> None:
        fixture = Fixture()
        self.addCleanup(fixture.close)
        wrong = fixture.repo / "envs" / "other"
        fixture.create_prefix(wrong)
        fixture.create_target()
        before = _snapshot_tree(wrong)
        result = fixture.run(prefix=wrong)
        self.assert_rejected(result)
        self.assertEqual(_snapshot_tree(wrong), before)

        incomplete = fixture.repo / "envs" / "zeron-dev-incomplete"
        incomplete.mkdir(parents=True)
        result = fixture.run(prefix=incomplete)
        self.assert_rejected(result)
        self.assertFalse((incomplete / "registry").exists())

    def test_active_cargo_locks_reject_cleanup(self) -> None:
        for lock_relative in (
            "target/debug/.cargo-lock",
            "target/debug/.cargo-build-lock",
            "target/debug/.cargo-artifact-lock",
            "envs/zeron-dev/.package-cache-mutate",
            "envs/zeron-dev/.package-cache",
        ):
            with self.subTest(lock_relative=lock_relative):
                fixture = self.fixture()
                with _held_exclusive_lock(fixture.repo / lock_relative):
                    before = _snapshot_tree(fixture.root)
                    result = fixture.run()
                    self.assert_rejected(result)
                    self.assertEqual(_snapshot_tree(fixture.root), before)
                fixture.close()

    def test_strip_failure_preserves_all_cleanup_inputs(self) -> None:
        fixture = self.fixture()
        fake_tools = fixture.root / "fake-tools"
        fake_tools.mkdir()
        fake_strip = fake_tools / "strip"
        fake_strip.write_text("#!/bin/sh\necho simulated strip failure >&2\nexit 77\n", encoding="utf-8")
        fake_strip.chmod(0o755)
        before = _snapshot_tree(fixture.root)

        result = fixture.run(path_prefix=fake_tools)
        self.assert_rejected(result)
        self.assertEqual(_snapshot_tree(fixture.root), before)

    def test_ldd_rejects_missing_or_deleted_runtime_library(self) -> None:
        cases = {
            "missing": "libmissing.so.1 => not found (0x0000000000000000)\n",
            "toolchain": None,
        }
        for name, output in cases.items():
            with self.subTest(name=name):
                fixture = self.fixture()
                fake_tools = fixture.root / "fake-tools"
                fake_tools.mkdir()
                if output is None:
                    toolchain_library = (
                        fixture.prefix
                        / "share/rustup/toolchains/stable/lib/libzeron-test.so"
                    )
                    output = f"libzeron-test.so => {toolchain_library} (0x0000000000000000)\n"
                fake_ldd = fake_tools / "ldd"
                fake_ldd.write_text(
                    "#!/bin/sh\nprintf '%s' "
                    + repr(output)
                    + "\n",
                    encoding="utf-8",
                )
                fake_ldd.chmod(0o755)
                before = _snapshot_tree(fixture.root)

                result = fixture.run(path_prefix=fake_tools)
                self.assert_rejected(result)
                self.assertEqual(_snapshot_tree(fixture.root), before)
                fixture.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("-q", "--quiet", action="store_true", help="pass unittest's quiet flag")
    args = parser.parse_args()
    if not PRODUCTION.is_file():
        print(f"production script is not available yet: {PRODUCTION}", file=sys.stderr)
        return 2
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(CleanupAfterBuildTests)
    result = unittest.TextTestRunner(verbosity=0 if args.quiet else 2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
