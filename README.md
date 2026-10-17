# Codex Deck · 多会话 Codex TUI

基于 `codex app-server` 的独立终端工作台。左侧跨目录找会话，中间继续任务，`/side` 或 `/btw` 在右侧打开持久化侧对话。切换会话或退出界面后，任务由自己的后台服务继续持有。

## 启动

运行环境为 Linux，需要 stable Rust、C 编译器，以及已安装、已登录并支持 `app-server` 的 Codex CLI。测试还需要 Python 3。SQLite 随程序构建，无需额外数据库服务。

```sh
git clone https://github.com/chengfengke/codex-session-deck.git
cd codex-session-deck
sh scripts/build.sh
./codex-deck
```

仓库保存源代码，构建结果生成在 `bin/`。启动器按 CPU 选择对应程序；缺少程序且已安装 Cargo 时会自动构建。也可将命令安装到 Cargo 的 bin 目录，在任意工作目录启动：

```sh
cargo install --path . --locked
codex-deck
```

请将 Cargo 的 bin 目录加入 `PATH`。直接安装的命令采用用户 XDG 状态目录；项目启动器默认采用项目的 `.state/`。如需两种入口共享后台和数据，请在两种入口中设置相同的 `CODEX_DECK_HOME`，并在迁移前关闭空闲后台。

默认使用 `PATH` 中的 `codex`，也会检测用户本地 bin 目录。可通过环境变量选择 CLI 或状态目录，例如：

```sh
CODEX_DECK_CODEX="$(command -v codex)" ./codex-deck
CODEX_DECK_HOME="$PWD/.state/custom" ./codex-deck
```

首次进入后，按 `Ctrl+G`，松开，再按 `n` 创建会话草稿。填写执行目录，`Tab` 切换字段，`Enter` 确认。第一条消息发送时才在 Codex 中建立 thread。左侧按工作目录分组，点击会话可直接切换，点击目录可展开或折叠。按 `Ctrl+G p` 聚焦左侧内嵌筛选框，输入标题、会话 ID 或目录，按上下键选择、Enter 打开；搜索不会改动会话草稿。按 Tab 返回对话。

```text
/side 检查一下当前方案的边界情况
/btw 解释一下这里为什么需要锁
```

两条命令是同一种侧对话入口。只输入 `/side` 或 `/btw` 会打开空侧对话供继续输入。默认从最近一个已经完成的 turn 分叉，使用父会话的目录与模型，并启用只读讨论。父任务可以继续运行。父会话尚无完整 turn 时，会以首条请求的显式引用建立独立讨论。

侧对话内使用 `/send-parent`，可将最近一条 Codex 回答放进父会话草稿，编辑并发送。也支持 `/send-parent 自己整理的结论`。`/parent` 返回父会话，`/promote` 将侧对话设为独立显示。

## 常用操作

`Ctrl+G` 是前缀键：按下后松开，再按后面的键，无需同时按三键。

| 操作 | 按键 / 命令 |
| --- | --- |
| 新会话 / 左侧筛选与切换 | `Ctrl+G n` / `Ctrl+G p`；也可直接点击左栏 |
| 独立快速切换器（可选） | `Ctrl+G f` 或 `/quick-switch` |
| 打开侧对话 | `/side [问题]`、`/btw [问题]`、`Ctrl+G s` |
| 完全收起 / 展开右栏 | `/side-panel`、`Ctrl+G o` 或顶部 Hide / Show side |
| 全局消息内容搜索 | `/find [关键词]`、`Ctrl+G /` 或顶部 Find messages |
| 返回最新消息 | 搜索上下文中 `/live` 或点击上下文提示 |
| 后台任务总览 | `/tasks`、`Ctrl+G t` 或顶部 Tasks |
| 发送 / 换行 | `Enter` / `Ctrl+J`；粘贴保留多行 |
| 任务运行中继续发送 | `Enter` 将消息排入该会话的队列 |
| 明确改变当前任务方向 | `/steer 新的要求` |
| 停止当前任务 | `Ctrl+C` 或 `/stop` |
| 聚焦窗格 | `Ctrl+G` 后按方向键或 `h j k l` |
| 切换标签 | `Tab` / `Shift+Tab`，或 `Ctrl+G [` / `]` |
| 键盘移动 / 停靠 | `Ctrl+G m`，上下选择目标、左右选择方向，Enter 应用 |
| 键盘缩放分屏 | `Ctrl+G r`，方向键调整，Enter / Esc 结束 |
| 最大化 / 恢复 | `Ctrl+G z` |
| 关闭视图 / 撤销布局 | `Ctrl+G w` / `Ctrl+G u` |
| 待处理审批 / 提问 | `Ctrl+G a` 或顶部 Approvals；PgUp/PgDn 阅读长请求 |
| 改动 / 侧栏 / 帮助 | `Ctrl+G d` / `Ctrl+G b` / `F1` |
| 阅读历史 | 鼠标滚轮、PgUp/PgDn；滚动后不会被新输出拉回底部 |
| 退出界面，保留后台任务 | `Ctrl+G q`、`Ctrl+D` 或 `/quit` |

鼠标可将侧栏会话或顶部标签拖到窗格四边以分屏，拖到中心或另一标签上合并为标签组。拖动分隔线调整比例，拖动侧栏边界调整宽度，侧栏行之间拖动可重排。默认并排显示左侧会话管理、中间主会话和右侧侧对话。尚未打开侧对话时，右侧保留入口；`/side`、`/btw` 打开右侧会话，后续侧对话进入该栏的标签组。在左侧选择侧对话时，仍保留中间的父会话。自定义布局最多四个可见聊天窗格。80、100、160 列都保留默认三栏，极窄终端才临时收为标签；放大后恢复布局。

`/side-panel hide` 完全收起右栏，主对话占据空出的宽度；`/side-panel show` 恢复保存的布局。收起不会关闭侧对话或停止任务，草稿与标签均保留，状态按主会话保存。关闭最后一个侧对话视图也会收起右栏；再次打开侧对话会自动展开。

推荐 160×40 字符，80×24 也能切换和使用主要功能。终端需启用 UTF-8 与鼠标报告。默认捕获鼠标；使用终端自身的修饰键（通常是 Shift）进行原生文本选择。

## 其他命令

- `/new [目录]`、`/search`（左侧内嵌筛选）、`/quick-switch`（可选弹窗）、`/archived`（左栏归档记录）、`/rename [名称]`、`/pin`
- `/project 名称`：为当前会话添加项目标签，可用于筛选；左栏仍以工作目录分组，不扩大文件权限。
- `/model` 或 `/models`：打开模型选择列表，上下键或 Tab 移动，Enter 或点击确认，输入文字筛选，Esc 取消。显示当前模型和默认模型，较长列表可用 PgUp/PgDn 或滚轮浏览。`/model ID` 可直接指定模型；自定义服务未列出的 ID 也可手动设置。修改只作用于当前会话的后续任务，运行或排队时禁止修改。
- `/cwd 目录`：为下一个任务选择执行目录；运行或排队时禁止修改。
- `/permissions read-only`、`/permissions workspace-write`：切换讨论与执行；运行或排队时禁止修改。
- `/queue`、`/queue clear`、`/queue release`：查看、清除或在确认后重新释放保留队列。
- `/history`：后台加载更早 30 轮历史（后端提供分页历史时）。首次打开先加载最近 8 轮，读取期间可以继续切换会话。
- `/diff`：查看当前 turn diff 和工作区 `git diff HEAD`；工作区 diff 包含所有来源的变动。
- `/worktree 绝对目录 已存在的ref`：创建 detached Git worktree；随后在该目录新建会话，清理由用户管理。
- `/archive`：确认后归档；已知存在未归档分叉时阻止操作。`/restore` 恢复。
- `/login`：启动 Codex device login，显示验证网址与验证码；已有 CLI 登录会直接复用。
- `/refresh`：刷新全局索引。`/reconnect`：恢复失去连接的 Codex 后端。
- `/close`、`/move`、`/help`。

直接发送以斜线开头的文本，请使用 `//` 前缀，例如 `//api/login 返回什么`。

## 后台与状态

`/tasks` 展示本后台已知的运行中、排队、待审批 / 待回答、失败 / 状态未知的会话。Tab / Shift+Tab 切换状态筛选，输入文字筛选标题、目录、ID 或状态，上下键选择，Enter 或点击进入会话。F2 进入所选会话的待处理请求。每行显示队列数量、held 数量、请求数量和工作目录；底部预览下一条排队消息。任务按需关注、异常、运行、排队的优先级分类，同一会话只出现一次。浏览总览不会停止任务或放行队列；已完成且无排队消息的会话不列入。外部 CLI / App 独立持有的实时任务不属于本后台总览。

```sh
./codex-deck status
./codex-deck shutdown
```

两条命令只连接已经启动的服务。`shutdown` 在仍有运行或等待批准的任务时会拒绝关闭。退出 TUI、关闭标签、停止任务和归档是不同的操作。

兼容原 `sessiondesk` 与 `start.sh` 启动入口、`SESSIONDESK_HOME` / `SESSIONDESK_CODEX` 环境变量；新旧名称共用同一后台与数据。新的 `CODEX_DECK_*` 变量优先。状态目录中的 `sessiondesk` 名称保留以兼容已有数据。

直接运行二进制时，状态保存在 XDG 用户状态目录下的 `sessiondesk/<Codex-home指纹>/`；项目启动器默认将 `XDG_STATE_HOME` 设为项目内的 `.state/`，含 SQLite 数据库、后台日志与 Unix socket。目录权限为 0700，socket 为 0600。不同 `CODEX_HOME` 默认使用独立状态空间。可通过 `CODEX_DECK_HOME` 指定独立状态目录；同一目录只能对应一个后端配置。

选中历史会话时只读获取记录，不获取执行写锁；已有 CLI 占用会话时也可浏览历史。明确发送任务时才恢复执行连接；如果该会话仍被其他进程占用，发送会返回写锁冲突，不自动转移或抢占。

Codex 负责对话与执行；本程序保存布局、项目、置顶、草稿、滚动位置、未读、侧对话关系、队列，以及本地消息搜索索引。登录凭据仍由 Codex 管理。每个会话按 **thread.id** 分流，分叉共享的 sessionId 不用于消息路由。

后台连接异常后，可能已经提交的消息标记为 uncertain，不自动重发。后台服务重启后，未处理队列变为 held。先检查历史确认执行情况，再使用 `/queue release` 明确放行，或 `/queue clear` 清除。已运行的任务只在后台进程仍存活时继续；机器重启、服务崩溃或明确 shutdown 会终止其运行时。

同一或重叠工作区的写任务由本程序串行调度，只读讨论和不同工作区可并行，最多同时启动四个任务。该协调只覆盖本服务发起的任务，外部 CLI 或 App 的写入需要使用不同 worktree 隔离。

## 全局消息内容搜索

`/find 关键词` 搜索后端可读取的全部会话消息和工具文本，包含归档会话及最近缓存之外的历史。`/find` 或 `Ctrl+G /` 打开搜索框，输入后 Enter 搜索；上下键 / PgUp / PgDn / 滚轮选择结果，再按 Enter 或点击打开命中消息附近的历史上下文。上下文独立于实时消息缓存，使用 `/live` 或点击提示返回最新消息；正常发送消息也会返回实时会话。

搜索按原文子串匹配，不区分大小写，不解释正则或通配符。首次搜索在后台分页建立索引，显示进度并逐步提供结果；后续复用未变化的历史。最多显示 100 条命中，缩小关键词范围可查看其他结果。Ctrl+R 刷新，Esc 取消；读取失败的历史会显示错误数量，避免将不完整结果当作完整搜索。搜索期间可取消后继续使用其他功能。

索引位于本程序状态目录的 `message-search.sqlite`，保存已读取的消息文本，文件权限为 0600，目录权限为 0700。搜索只读历史，不恢复执行写锁，也不提交模型请求。未发送草稿不进入搜索。搜索范围取决于所选 Codex home 和后端能够列出、读取的记录；不涵盖其他账号、其他机器或未落盘的实时输出。同一后台一次运行一个搜索，新搜索替换前一次。

## Linux 构建

```sh
sh scripts/build.sh
./codex-deck
```

脚本从自身位置定位项目根目录，可在其他工作目录调用。构建产物为 `bin/codex-deck-linux-<arm64|x64>`，兼容入口仍为 `sessiondesk` 与 `start.sh`。构建尊重调用者的 Rust 和 C 编译器配置；`scripts/build-server.sh` 可复用项目内已有的私有工具链，或使用系统工具链。

## 验证

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 tests/integration.py
python3 tests/responsiveness.py
python3 tests/manager_pty.py
python3 tests/model_picker_pty.py
python3 tests/search_integration.py
python3 tests/workspace_pty.py
python3 tests/rename_compat.py
python3 tests/pty_smoke.py
# 已安装 Codex CLI 时，执行无模型请求的真实协议联通测试：
python3 tests/real_smoke.py
```

`tests/fake_codex.py` 只用于确定性集成验证，正常启动始终连接真实 Codex。`--demo` 是明确标记的离线界面演示，`--render-demo` 输出字符预览。

验证覆盖 Linux x86_64 上的编译、单元测试、Unix socket / SQLite / JSON-RPC 集成、PTY 终端操作，以及真实 Codex 的握手、模型列表和跨目录会话列表。真实后端测试只读历史和配置，不提交模型任务。版本为 0.1.0 开发版。

历史排版按消息缓存，只绘制可见行；连续输入合并处理，空闲时不重复重画。TUI 订阅增量快照，编辑草稿和布局不再重复传输全部会话历史。首次加载及继续分页在后台并行执行，显示 `loading history…`，不会阻止切换其他会话。

可用 `cargo test --release large_history_render_benchmark -- --ignored --nocapture` 测量长历史排版，或 `python3 tests/pty_history_perf.py target/release/codex-deck` 测量真实终端中的长历史切换；两者使用合成数据和模拟后台。

## 当前边界

- 接入所选本地 Codex home 的历史；不会自动接入桌面 App 云端记录或其他 app-server 进程中的实时任务。
- 首版输入为文本；附件、图片粘贴、远程主机、云同步、语音、浏览器面板没有实现。
- 工具结果以文本块呈现；通用 MCP elicitation 允许查看请求并手动填写 JSON 响应，尚无按任意 JSON Schema 自动生成的表单。
- 会话消息缓存保留最近窗口，完整历史由 Codex 持有；尚未验证万级会话性能。
- 原设计中的 GUI 级文件选取与工具卡片折叠尚未提供；可使用文本目录和系统终端选择功能。
- `docs/design-v1.md` 保留原设计目标；当前实际交付能力与验证范围以本 README 为准。

接口依据：[Codex app-server 官方文档](https://learn.chatgpt.com/docs/app-server) ；运行时需要提供对应接口的 Codex CLI。
