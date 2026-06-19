---
name: ui-verify
description: VoxInk GPUI 界面可视化验证——构建并启动应用、截图（可选先点击/滚动/切主题），用于确认 UI 改动的真实效果。当需要"看一眼界面""验证 UI 改动""截图主窗口/设置面板""暗色模式截图"时使用。
---

# VoxInk UI 可视化验证

`ui-shot.ps1` 一次调用完成全部步骤：杀进程 → 临时改 config（直接可见启动 + 目标主题）→ 可选构建 → 启动 → 可选点击/滚动 → 截图 → 强杀 → 还原 config。**所有参数一次给定，无需多步交互。**

## 用法

必须用 **Windows PowerShell 5.1**（`powershell.exe`，非 pwsh —— `System.Drawing` 不在 pwsh 中）：

```
powershell.exe -ExecutionPolicy Bypass -File .claude/skills/ui-verify/ui-shot.ps1 -Out target/shot/x.png [参数]
```

截完后用 Read 工具读回 PNG（会渲染图片）检查，再迭代。脚本输出 `CAPTURED:<path>` 表示成功；`WINDOW_NOT_FOUND` / `EXE_NOT_FOUND:` / `BUILD_FAILED` 为失败。

## 参数（全部可选，默认即可出主窗口浅色图）

| 参数 | 默认 | 说明 |
|---|---|---|
| `-Out` | `shot.png` | 输出 PNG 路径（建议 `target/shot/<名>.png`，该目录已 gitignore） |
| `-Theme` | `light` | `light`/`dark`/`system`/`keep`（keep=不改 config 主题） |
| `-Build` | 关 | 截图前先 `cargo build`（改了 Rust 代码时加上；失败即中止） |
| `-ClickX -ClickY` | -1 | 截图前在窗口相对坐标点一下（如打开设置面板） |
| `-ScrollX -ScrollY -ScrollTicks` | -1/0 | 截图前滚动（设置面板长内容） |
| `-WaitMs` | 6500 | 窗口出现后的等待，默认足够让启动期热键冲突 toast 自动消失 |

## 常用示例

```
# 改了 UI 代码后看主窗口（浅色）
powershell.exe -ExecutionPolicy Bypass -File .claude/skills/ui-verify/ui-shot.ps1 -Out target/shot/main.png -Build

# 暗色模式（无需手改 config 再还原，脚本自动处理）
powershell.exe -ExecutionPolicy Bypass -File .claude/skills/ui-verify/ui-shot.ps1 -Out target/shot/dark.png -Theme dark

# 打开设置面板（点标题栏齿轮，坐标按当前布局微调）后截图
powershell.exe -ExecutionPolicy Bypass -File .claude/skills/ui-verify/ui-shot.ps1 -Out target/shot/settings.png -ClickX 212 -ClickY 17
```

## 为什么这样设计（勿改坏这些点）

- **改 config 而非 force-show**：默认配置 `start_minimized=true`，应用启动即隐藏到托盘。脚本临时把 `start_minimized=false`（及 `theme`）写进 `%APPDATA%\VoxInk\config.toml`，窗口便直接以目标状态可见，省去隐藏/恢复、暗色切换/还原等多步操作。
- **强杀 + 备份还原双保险**：结束时 `Stop-Process -Force`（TerminateProcess 不触发 `on_app_quit`，绝不把临时配置写盘），再从 `config.toml.uishot.bak` 还原原配置。即使中途异常，`finally` 也会还原。
- **构建前先杀进程**：应用运行中无法 relink（`os error 5`），`-Build` 已内置先杀后建。
- **截图前置顶**：用 `SetWindowPos HWND_TOPMOST` 越过 IDE 等窗口（`SetForegroundWindow` 受前台锁限制可能无效），截完恢复 NOTOPMOST。
- **按窗口标题查找**：依赖窗口标题为 `VoxInk`（无边框窗口仍保留 `title: Some("VoxInk")` 元数据，故能查到）。

无麦克风的机器只能验证静态 UI；录音/转写等运行时行为仍需用户真机验证。
