# 历史切换性能

历史按消息缓存排版，只绘制可见行；连续输入合并处理，空闲时不重复重绘。界面订阅增量快照，草稿和布局变更不重复传输全部历史。

历史读取在后台执行，不阻塞界面切换。首次只加载最近 8 轮，`/history` 继续取更早 30 轮；完整记录由 Codex 持有，消息内容搜索另用分页索引。

## 复现

在项目根目录执行，使用当前环境的 Rust、C 编译器和 Python 3：

```sh
sh scripts/verify-performance.sh
```

只运行单项基准：

```sh
cargo test --release --locked large_history_render_benchmark -- --ignored --nocapture
python3 tests/pty_history_perf.py target/release/codex-deck
```

集成测试使用动态临时目录与模拟 Codex 后端，不提交真实模型任务。可设置 `CARGO_NET_OFFLINE=true` 复用已经缓存的依赖；兼容性测试可通过 `CODEX_DECK_LEGACY_BINARY` 选择旧程序。

PTY 基准测量终端内的渲染与会话切换，不包含 SSH 网络、实际历史文件读取或模型响应耗时。结果随终端大小和主机性能变化，应以本机输出为准。
