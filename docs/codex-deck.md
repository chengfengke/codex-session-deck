# Codex Deck 使用说明

构建与安装见 [README](../README.md) 和 [Linux 部署](../DEPLOYMENT.md)。全局命令为 `codex-deck`；从源码目录运行时使用 `./codex-deck`。

| 操作 | 命令 / 快捷键 |
| --- | --- |
| 新会话 | `Ctrl+G n` 或 `/new [目录]` |
| 查找、切换会话 | 点击左栏，或 `Ctrl+G p` 输入标题、目录、ID |
| 侧对话 | `/side [问题]` 或 `/btw [问题]` |
| 完全收起、恢复右栏 | `/side-panel` 或 `Ctrl+G o` |
| 关闭当前视图 | `/close` 或 `Ctrl+G w` |
| 消息内容搜索 | `/find [关键词]` 或 `Ctrl+G /` |
| 返回实时消息 | `/live` |
| 后台任务总览 | `/tasks` 或 `Ctrl+G t` |
| 选择模型 | `/model` 或 `/models` |
| 移动、停靠 | 拖动标签或左栏会话，或 `Ctrl+G m` |
| 调整分屏 | 拖动分隔线，或 `Ctrl+G r` |
| 退出界面并保留后台任务 | `Ctrl+G q`、`Ctrl+D` 或 `/quit` |
| 帮助 | `F1` 或 `/help` |

`Ctrl+G` 是前缀键，按下后松开，再按后面的键。按 `Enter` 发送，按 `Ctrl+J` 换行。运行中的会话继续发送消息时会排队；`/steer` 用于明确改变当前任务方向。

左侧按工作目录分组，中间显示主会话，右侧显示侧对话。侧对话默认使用父会话的目录与模型，并以只读模式讨论。`/send-parent` 将结论放进父会话草稿，检查后再发送。收起右栏、关闭视图和退出界面均不等于停止任务；`Ctrl+C` 或 `/stop` 才会停止当前任务。

状态由 `CODEX_DECK_HOME` 或 XDG 环境确定。旧 `SESSIONDESK_*` 变量和默认 `sessiondesk` 状态子目录保留兼容；切换程序名不会自动新建另一套会话。不同入口要复用同一后台时应选择相同状态目录。
