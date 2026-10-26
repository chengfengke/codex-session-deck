# Codex Deck 品牌资源

`logo.png` 是 README 使用的透明背景坐姿角色。终端启动与标题栏均只输出文字，不加载图片、不需要图片协议。

## 终端线条

- `startup-logo-unicode.txt`：Unicode Braille 点阵曲线，全身坐姿，供至少 48×32 的终端使用。
- `startup-logo-compact.txt`：小窗口精简版；不够容纳角色时仅显示启动文字。
- `startup-logo.txt`：ASCII 回退，设置 `CODEX_DECK_LOGO=ascii` 或 `TERM=dumb` 使用。
- 宽度足够时，名称、标语和连接提示显示在角色右侧；窄窗口显示在下面。
- `src/brand.rs` 中两行微型表情对应眼睛与微笑，占 5 列、2 行，没有头部边框或身体。按钮起点留到第 7 列。

启动显示与连接并行，至少停留 600 毫秒。`--help`、`--version`、`status`、`shutdown`、后台服务和 `--render-demo` 不打印启动图；`--demo` 直接展示带表情的工作台。

## 本地验证和字体

在 Linux 项目目录运行：

```sh
python3 scripts/preview-brand.py
python3 scripts/preview-brand.py --ascii
```

预览读取程序实际内嵌的字符资源，不启动后台或请求模型。资源由 `python3 scripts/generate-brand.py` 从几何路径生成，使用 Python 标准库，无需图片与字体处理依赖。

VS Code 的终端字体由本机而非 SSH 服务器控制。请使用本机已安装、覆盖 Braille Unicode 区段的等宽字体，例如 macOS 的 Menlo；在设置中搜索 `terminal.integrated.fontFamily`。其他系统需选择本机存在的字体。程序不能远程更改本机字体；不同字体的点形和行距有差别，不保证跨终端像素一致。若显示方框或错位，换字体或使用 ASCII 回退。

`NO_COLOR` 禁用 ANSI 颜色，但保留字符图形。无需开启 `terminal.integrated.enableImages`。
