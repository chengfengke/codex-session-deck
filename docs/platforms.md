# 跨平台运行

## 构建和启动

- Linux：Rust 1.89+、C 编译器；`sh scripts/build.sh`，然后 `./codex-deck`。Codex 的命令沙箱需要 bubblewrap（如 `sudo apt-get install bubblewrap`），缺少时命令会在沙箱内失败并转为审批请求。
- macOS：Rust 1.89+、Xcode Command Line Tools；同上。产物名包含 `macos-arm64` 或 `macos-x64`。
- Windows：MSVC Rust 工具链、Visual Studio C++ Build Tools；PowerShell 执行 `./scripts/build.ps1` 和 `./codex-deck.ps1`。也可直接运行 `bin/codex-deck-windows-x64.exe`，ARM64 名称相应替换。

三平台都需要安装并登录支持 `app-server` 的 Codex CLI。Windows 自动在 PATH 中查找 `codex.exe` 和 npm 的 `codex.cmd`，可用 `CODEX_DECK_CODEX` 指定完整路径。不要把多个命令或命令行参数放进该环境变量。

## 后台与状态

Linux/macOS 使用 Unix domain socket 和 `setsid`；Windows 使用命名管道和脱离控制台的独立后台进程，不依赖 WSL。三平台使用文件独占锁防止重复后台。

Windows 管道仅授权当前用户，拒绝远程访问，使用系统自带 `whoami.exe` 获取 SID。不打开 TCP 端口。默认数据目录基于 `%LOCALAPPDATA%/sessiondesk/<profile-hash>`；Linux/macOS 保留原来的 XDG 路径及兼容变量，不迁移既有数据。

Linux 项目启动器继续默认使用项目 `.state/`。Windows 和 macOS 使用用户状态目录，`CODEX_DECK_HOME` 可覆盖。Windows 状态文件继承目录的 ACL，自定义目录应由当前用户管理。

`--ipc-path` 输出本地 IPC 地址，仅用于诊断与测试。`status`、`shutdown` 不自动启动后台。

## 验证范围

新增 `tests/platform_smoke.py` 使用原项目模拟 Codex 加上原生可执行测试入口，覆盖本地 IPC、重复后台锁、消息往返、重新连接和退出，无真实模型请求。GitHub Actions 在 Linux/macOS/Windows 分别执行编译、单元测试与公共冒烟测试。

本次在 macOS 本机执行验证；Windows、Linux 的原生执行需由对应机器或配置的 CI 确认。不要把“已编写适配”视作“所有平台已实机通过”。Windows ARM64 同样需要该架构的实际构建环境，CI 默认 Windows runner 验证 x64。

字符 logo 的样式仍受本机终端字体影响，详细说明见 `assets/README.md`。
