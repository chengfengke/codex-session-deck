> Linux/macOS/Windows 的新增构建入口、后台实现及验证范围见 [docs/platforms.md](docs/platforms.md)。下文原有 Linux 部署流程继续适用。

# Linux 部署

## 前置条件

- Linux，UTF-8 交互终端；推荐 160×40，80×24 可使用核心功能。
- stable Rust 和 C 编译器；SQLite 通过 bundled 构建。
- 已安装并登录支持 `app-server` 的 Codex CLI。
- 执行测试时需要 Python 3。

## 构建和启动

```sh
git clone https://github.com/chengfengke/codex-session-deck.git
cd codex-session-deck
sh scripts/build.sh
./codex-deck
```

启动器根据 CPU 选择 `bin/` 中的原生程序。缺少产物但存在 Cargo 时会构建。`scripts/build-server.sh` 可使用项目内已有的 `.runtime/` 工具链，也支持系统工具链；仓库不包含私有工具链。

也可以直接安装命令：

```sh
cargo install --path . --locked
codex-deck
codex-deck status
```

安装后需要将 Cargo 的 bin 目录加入 `PATH`。远程使用时，先通过 SSH 登录主机，再运行命令。

## 环境与数据

| 变量 | 作用 |
| --- | --- |
| `CODEX_DECK_CODEX` | 选择 Codex CLI；默认从 `PATH` 查找 |
| `CODEX_DECK_HOME` | 选择本程序的独立状态目录 |
| `CODEX_HOME` | 选择 Codex 配置、账号及历史空间 |
| `XDG_STATE_HOME` | 未指定独立状态目录时的状态根目录 |

项目启动器默认在项目 `.state/` 中保存状态；直接运行安装后的二进制使用用户 XDG 状态目录。若需要切换入口并复用数据，给两种入口设置同一个 `CODEX_DECK_HOME`。支持旧的 `SESSIONDESK_HOME` 和 `SESSIONDESK_CODEX`，新变量优先。默认状态子目录中的 `sessiondesk` 名称用于兼容已有数据库。

例如选择项目内独立状态：

```sh
CODEX_DECK_HOME="$PWD/.state/custom" ./codex-deck
```

程序不替换原有 Codex CLI，也不改写其登录凭据。`bin/`、`target/`、`.runtime/`、`.state/`、数据库、日志、环境文件与凭据不进入版本控制。

## 更新

先在独立工作副本完成构建和验证。若后台正在执行任务，保留现有进程，等空闲后再重启；界面退出并不停止后台任务。

```sh
./codex-deck status
./codex-deck shutdown
```

`shutdown` 在仍有运行或待批准任务时拒绝关闭。更新前应额外检查队列并备份所选状态目录；后台重启后的队列保持 held，检查历史后再手动释放。

验证方法见 [README](README.md) 和 [性能说明](PERFORMANCE.md)。
