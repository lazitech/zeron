# Zeron

Control your coding agents (Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, Antigravity) locally by default, with optional multi-device sync.

*English | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![Zeron desktop app](docs/media/readme/app-screenshot.jpg)

## Desktop app

Download the latest release for your platform from [GitHub Releases](https://github.com/zeronsh/zeron/releases/latest):

- **macOS** — `zeron-<version>-macos-arm64.dmg`
- **Windows** — `zeron-<version>-windows-x86_64-setup.exe`
- **Linux** — `zeron-<version>-linux-<arch>.tar.gz`, then run its `install.sh`

No account or network connection is needed; sessions stay on your device. The app updates itself.

## Headless (CLI)

For servers and other machines without a display, such as a VPS that keeps agents running after you close your laptop. Linux only:

```bash
curl -fsSL https://zeron.sh/install.sh | sh
zeron status
```

The installer starts the engine as a background service that survives reboots.

```bash
zeron status      # local/synced mode and engine status
zeron update      # update to the latest release
zeron daemon start|stop|restart|status
```

## Multi-device sync (optional)

Sign in to start an agent on one device and follow or drive it from another:

```bash
zeron daemon stop
zeron login        # or: zeron logout to return to local-only
zeron daemon start
```

Devices signed in to the same account can read and write each other's workspace files, so only sign in devices you trust. Existing local sessions are never uploaded.

## 编译后的空间清理（Linux / zeron-dev）

已写好：[scripts/cleanup-after-build.py](scripts/cleanup-after-build.py)。默认保留可运行的 Zeron，删除编译中间文件、Cargo 缓存和 Rust 工具链，并去掉程序调试信息。

编译完成后，在仓库根目录运行：

```sh
./scripts/cleanup-after-build.py --dry-run  # 预览
./scripts/cleanup-after-build.py           # 清理
```

本机验证记录（2026-10-03）：程序副本实测从约 **1.95 GiB 缩到 294 MiB**，清理后 `target` 预计约 300 MiB。已通过 11 项隔离测试，**尚未实际清理你的目录**。

下次编译前恢复工具链：

```sh
source /home/lazic/miniconda3/etc/profile.d/conda.sh
conda activate zeron-dev
./scripts/bootstrap-conda-rust.sh
cargo build -p zeron --locked
```

清理后仍可直接运行 `zeron`；下次编译会重新下载依赖并完整构建。更多清理选项见 [Conda 开发环境说明](docs/development/conda.md#reclaim-space-after-a-desktop-build)。

## Sponsors

Thank you to [The Context Company](https://www.thecontextcompany.com/) for sponsoring Zeron. You can help fund Zeron's development too by [becoming a sponsor on GitHub](https://github.com/sponsors/zeronsh).

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/zeronsh/zeron) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
