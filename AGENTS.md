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
- `./scripts/cleanup-after-build.py` retains the desktop executable but removes
  build intermediates, Cargo downloads, and the Conda Rust toolchain by default.
  After cleanup, launch with `zeron` directly. Before running Cargo again,
  activate `zeron-dev` and run `./scripts/bootstrap-conda-rust.sh` to restore Rust.
  See `docs/development/conda.md` for preview and optional retention flags.
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
  user's fork. Future requested upstream updates include pushing the verified
  result to that fork under the workflow below.
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
  Later upstream updates must follow the requested workflow below, preserving
  personal changes and inspecting the resulting diff. Do not start an upstream
  update without a user request.

## 以后检查官方更新的完整流程

用户已授权：以后提出“检查更新”“更新官方版本”或“合并官方更新”时，默认完成
**检查更新 → 保留本地全部功能并合并 → 编译验证 → 运行脚本清理空间 → 推送到个人 GitHub fork**。
此授权包括必要的本地保存提交、合并提交、实际运行清理脚本和推送个人分支，
无需逐步再次确认。当次请求若明确限定“只检查”等范围，则遵循当次请求。
没有更新请求时，不自行启动这套流程。

### 1. 检查仓库状态与官方更新

- 在 `/home/lazic/zeron` 操作，先查看当前分支、工作区状态和远程地址。
  当前个人分支为 `feature/statistics-dashboard`。
- 官方远程 `origin`：`https://github.com/zeronsh/zeron.git`。
  个人远程 `fork`：`git@github.com:lazitech/zeron.git`，对应
  [lazitech/zeron](https://github.com/lazitech/zeron)。确认实际地址仍匹配后再操作。
- 获取官方 `main`、标签和个人远程分支，并比较官方提交与本地 `HEAD`：

  ```sh
  git status --short
  git branch --show-current
  git remote -v
  git fetch origin --tags
  git fetch origin main:refs/remotes/origin/main
  git fetch fork
  git log --oneline HEAD..origin/main
  git rev-list --count HEAD..origin/main
  ```

- 记录本轮官方目标提交、版本、新增提交数和主要变更。
  若官方提交已经全部包含在本地，报告“无新更新”，结束更新流程。
- 合并前妥善保存所有未提交、未跟踪的本地开发改动，按明确文件范围创建本地保存提交；
  不适合提交的文件另行备份并在合并后恢复。不要丢弃或覆盖用户改动。
- 以保存后的个人分支创建带日期和提交号的 `backup/personal-before-upstream-*`
  本地备份分支，记录合并前的提交和功能清单。

### 2. 合并到本地，保留全部个人功能

- 使用 Git merge 合并 `origin/main`，保留个人分支历史，不用 rebase 重写它。
  如个人 fork 有本地尚未包含的提交，也先检查并合并，保留双方改动。
  备份完成、工作区妥善保存后，合并官方目标：

  ```sh
  git merge --no-ff origin/main
  ```

- 合并前检查上次官方基线到本地的差异，以及本轮官方变更，逐项识别个人功能。
  必须保留**全部本地开发功能**，不能只保留统计页面或仅处理发生冲突的文件。
  已知个人功能包括统计仪表盘与热力图、会话 Token 计数、原生历史统计读取、
  Pi 历史与上下文桥接、全窗口背景和鼠标导航；这份列表不替代实际差异检查。
- 冲突按功能意图逐项整合，官方重构后将个人行为迁移到新接口。
  不要对整批冲突直接选 `ours` / `theirs`，也不要为通过编译而删除个人功能。
- 检查合并结果，包含没有文本冲突的行为变化；保留本地设置、数据兼容性和
  Conda 构建配置。修复回归后再继续，不能将功能缺失的版本作为完成结果。

### 3. 恢复开发环境、编译并验证

- 激活隔离的 `zeron-dev` 环境。如果上次清理删除了工具链，或新依赖需要更新
  Rust，先运行 bootstrap；环境缺失时才运行完整 setup。始终使用 Conda 内的
  Cargo/Rust，避免再次用系统工具链触发 `cold_path` 等版本错误。

  ```sh
  source /home/lazic/miniconda3/etc/profile.d/conda.sh
  conda activate zeron-dev
  ./scripts/bootstrap-conda-rust.sh
  cargo build --locked
  ```

- 核对实际 `cargo` / `rustc` 路径和版本，以及系统 `pkg-config`、OpenSSL 配置。
  编译失败时修复根因并重新编译，不跳过失败继续清理或推送。
- 在清理前运行与本轮改动和个人功能相关的测试。涉及这些核心模块时可运行：

  ```sh
  cargo test --locked -p zeron-doc -p zeron-engine -p zeron-harness -p zeron-ui --lib
  ```

- 原生 agent / Pi 桥接改动还需执行对应集成测试；界面行为改动需启动桌面检查
  相关个人功能。清理脚本自身改动时运行 `python3 scripts/test-cleanup-after-build.py`。
  根据实际变更选择必要检查；测试失败需调查、修复或有证据地确认环境原因。
- 确认 `./target/debug/zeron --version` 输出本轮预期版本，检查最终差异和
  `git diff --check`，确保个人功能保留完整。

### 4. 编译验证成功后，实际清理空间

- 等所有 Cargo/Rust 构建和测试进程退出，先预览，再实际执行：

  ```sh
  ./scripts/cleanup-after-build.py --dry-run
  ./scripts/cleanup-after-build.py
  ./target/debug/zeron --version
  ```

- 使用脚本默认范围：保留可运行桌面程序，去掉调试信息，删除编译中间文件、
  Cargo 下载缓存和 Conda 内的 Rust 工具链。保留 Rustup 安装入口、Conda 基础环境
  和 Zeron 用户数据。不要对保留程序再运行 `cargo clean`。
- 核对清理成功、程序仍能运行，并记录释放空间和剩余 `target` 大小。
  清理后使用 `zeron` 直接启动；下次构建前重新激活环境并运行 bootstrap。
  若旧程序仍在运行，退出并重新启动后，其旧可执行文件占用的空间才会释放。

### 5. 提交并推送到个人 GitHub fork

- 更新本文件的最近官方合并记录（提交、版本、日期、备份分支），将需要保留的
  个人开发改动、合并修复、脚本和说明整理成明确范围的提交。
- 检查提交内容和工作区，确认本轮变更已提交，然后显式推送到个人远程：

  ```sh
  git push fork HEAD:refs/heads/feature/statistics-dashboard
  ```

- 上述目标适用于当前个人分支。若用户以后改变个人开发分支，核实对应 fork
  分支后调整目标。不要推送到官方 `origin`，不要开上游 PR，也不要 force push。
- 推送若被远程新提交拒绝，获取并检查这些提交，合并、重新编译验证和清理后
  再推送。不要覆盖远程历史。
- 核对 fork 目标分支的远程提交号与本地 `HEAD` 一致，再报告完成：官方版本与
  合并提交、个人功能保留情况、编译测试结果、实际清理结果、fork 分支和提交。
  任一步失败时先处理失败；无法解决时说明具体原因和未完成步骤，不宣称全流程完成。

## Verification

- Automated builds and relevant tests are required during the requested update
  workflow above, as well as when the user separately asks for verification.
- Ubuntu's OpenSSL development files are extracted under
  `$CONDA_PREFIX/share/ubuntu-openssl`; environment-scoped `OPENSSL_*`
  variables select these headers and the system OpenSSL runtime libraries.
  See the administrator-free setup in `docs/development/conda.md`.
- For a requested visual check, activate `zeron-dev` and run `cargo run -p zeron`
  from the repository root.
