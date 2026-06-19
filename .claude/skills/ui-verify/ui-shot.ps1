<#
.SYNOPSIS
  VoxInk UI 可视化验证：一次性 杀进程 → 改配置 → (可选)构建 → 启动 → (可选)点击/滚动 → 截图 → 强杀 → 还原配置。

.DESCRIPTION
  设计为单次调用、自带所有参数，最小化 LLM 交互。
  - 通过临时改写 config.toml 的 start_minimized=false / theme，让窗口直接以目标主题可见启动，
    省去"隐藏到托盘再 force-show / 暗色再还原"的多步操作。
  - 结束时**强杀**进程（TerminateProcess 不触发 on_app_quit，绝不持久化临时配置），再从备份还原 config.toml，双保险。
  - System.Drawing 仅存在于 Windows PowerShell 5.1，因此本脚本必须用 powershell.exe（非 pwsh）运行。

.EXAMPLE
  powershell.exe -ExecutionPolicy Bypass -File ui-shot.ps1 -Out shot.png
  powershell.exe -ExecutionPolicy Bypass -File ui-shot.ps1 -Out dark.png -Theme dark -Build
  # 打开设置面板（点齿轮）后截图：
  powershell.exe -ExecutionPolicy Bypass -File ui-shot.ps1 -Out settings.png -ClickX 212 -ClickY 17
#>
param(
    [string]$Out = "shot.png",                 # 输出 PNG 路径（相对则相对当前目录）
    [ValidateSet("light","dark","system","keep")]
    [string]$Theme = "light",                  # 临时主题；keep=不改 config 的 theme
    [int]$ClickX = -1,                          # 截图前窗口相对坐标点击（-1=不点）
    [int]$ClickY = -1,
    [int]$ScrollX = -1,                         # 截图前滚动位置（配合 -ScrollTicks）
    [int]$ScrollY = -1,
    [int]$ScrollTicks = 0,                      # 向下滚动格数（0=不滚）
    [int]$WaitMs = 6500,                        # 窗口出现后渲染/启动 toast 自消失的等待（默认覆盖热键冲突 toast）
    [switch]$Build,                             # 截图前先 cargo build（失败则中止）
    [string]$ExePath = "",                      # 默认据脚本位置推导 <repo>/target/debug/VoxInk.exe
    [string]$ConfigPath = ""                    # 默认 %APPDATA%\VoxInk\config.toml
)

$ErrorActionPreference = "Stop"

# --- 路径推导：脚本在 <repo>/.claude/skills/ui-verify/ ---
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
if (-not $ExePath)    { $ExePath = Join-Path $repo "target\debug\VoxInk.exe" }
if (-not $ConfigPath) { $ConfigPath = Join-Path $env:APPDATA "VoxInk\config.toml" }
$backup = "$ConfigPath.uishot.bak"

function Kill-VoxInk {
    Get-Process VoxInk -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
}

# --- 1. 杀掉运行中的实例（构建 relink / 干净启动都需要）---
Kill-VoxInk

# --- 2. 可选构建（必须在杀进程之后，否则 os error 5 无法删 exe）---
if ($Build) {
    Push-Location $repo
    try { cargo build 2>&1 | Write-Output; if ($LASTEXITCODE -ne 0) { Write-Output "BUILD_FAILED"; exit 1 } }
    finally { Pop-Location }
}
if (-not (Test-Path $ExePath)) { Write-Output "EXE_NOT_FOUND:$ExePath"; exit 1 }

# --- 3. 备份并临时改写 config.toml（直接可见启动 + 目标主题）---
$patched = $false
if (Test-Path $ConfigPath) {
    Copy-Item $ConfigPath $backup -Force
    $cfg = Get-Content $ConfigPath -Raw
    $cfg = $cfg -replace '(?m)^\s*start_minimized\s*=\s*\w+', 'start_minimized = false'
    if ($Theme -ne "keep") {
        if ($cfg -match '(?m)^\s*theme\s*=') { $cfg = $cfg -replace '(?m)^\s*theme\s*=\s*".*?"', "theme = `"$Theme`"" }
    }
    Set-Content $ConfigPath $cfg -NoNewline -Encoding UTF8
    $patched = $true
} else {
    Write-Output "CONFIG_NOT_FOUND:$ConfigPath (首启将用默认配置，可能隐藏到托盘)"
}

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Text;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public class UiShot {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, IntPtr e);
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    public static IntPtr Find(string title, string procName) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, p) => {
            StringBuilder sb = new StringBuilder(256);
            GetWindowText(h, sb, 256);
            if (sb.ToString() == title && IsWindowVisible(h)) {
                uint pid; GetWindowThreadProcessId(h, out pid);
                try {
                    var proc = System.Diagnostics.Process.GetProcessById((int)pid);
                    if (proc.ProcessName.Equals(procName, StringComparison.OrdinalIgnoreCase)) { found = h; return false; }
                } catch {}
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static void Front(IntPtr h) {
        ShowWindow(h, 9); ShowWindow(h, 5);                       // SW_RESTORE / SW_SHOW
        SetWindowPos(h, new IntPtr(-1), 0, 0, 0, 0, 0x43);        // TOPMOST 越过 IDE
        SetForegroundWindow(h);
    }
    public static void Click(IntPtr h, int rx, int ry) {
        RECT r; GetWindowRect(h, out r);
        SetCursorPos(r.Left + rx, r.Top + ry);
        System.Threading.Thread.Sleep(120);
        mouse_event(0x0002, 0, 0, 0, IntPtr.Zero);
        mouse_event(0x0004, 0, 0, 0, IntPtr.Zero);
    }
    public static void Scroll(IntPtr h, int rx, int ry, int ticks) {
        RECT r; GetWindowRect(h, out r);
        SetCursorPos(r.Left + rx, r.Top + ry);
        System.Threading.Thread.Sleep(120);
        for (int i = 0; i < ticks; i++) { mouse_event(0x0800, 0, 0, unchecked((uint)(-120)), IntPtr.Zero); System.Threading.Thread.Sleep(60); }
    }
    public static void Capture(IntPtr h, string path) {
        RECT r; GetWindowRect(h, out r);
        int w = r.Right - r.Left, ht = r.Bottom - r.Top;
        Bitmap bmp = new Bitmap(w, ht, PixelFormat.Format32bppArgb);
        using (Graphics g = Graphics.FromImage(bmp)) { g.CopyFromScreen(r.Left, r.Top, 0, 0, new Size(w, ht)); }
        SetWindowPos(h, new IntPtr(-2), 0, 0, 0, 0, 0x43);        // NOTOPMOST 还原
        bmp.Save(path, ImageFormat.Png);
    }
}
"@

try {
    # --- 4. 启动 ---
    Start-Process $ExePath
    # --- 5. 轮询等待窗口出现（最多 ~20s）---
    $h = [IntPtr]::Zero
    for ($i = 0; $i -lt 40; $i++) {
        $h = [UiShot]::Find("VoxInk", "VoxInk")
        if ($h -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($h -eq [IntPtr]::Zero) { Write-Output "WINDOW_NOT_FOUND (是否隐藏到托盘？确认 config 已 start_minimized=false)"; exit 1 }

    [UiShot]::Front($h)
    Start-Sleep -Milliseconds $WaitMs                              # 等渲染 + 启动 toast 自动消失

    if ($ClickX -ge 0) { [UiShot]::Click($h, $ClickX, $ClickY); Start-Sleep -Milliseconds 600 }
    if ($ScrollTicks -gt 0) { [UiShot]::Scroll($h, $ScrollX, $ScrollY, $ScrollTicks); Start-Sleep -Milliseconds 400 }

    [UiShot]::Capture($h, $Out)
    Write-Output "CAPTURED:$Out"
}
finally {
    # --- 6. 强杀（不持久化临时配置）+ 还原 config.toml ---
    Kill-VoxInk
    if ($patched -and (Test-Path $backup)) {
        Move-Item $backup $ConfigPath -Force
    }
}
