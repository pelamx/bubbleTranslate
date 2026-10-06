# Draws the Microsoft Store listing screenshots, by driving the real app.
#
#   powershell -ExecutionPolicy Bypass -File windows\make-store-shots.ps1
#
# Partner Center wants at least one screenshot, 1366x768 or larger. These are
# 1920x1080. They are made by launching the built binary, opening a German
# paragraph in Notepad and selecting a sentence, so what the listing shows is
# the app actually translating rather than a mock-up.
#
# Two things are deliberately cropped out, and both are why the capture is not
# simply the whole screen: Notepad's tab strip carries the names of whatever
# files are open on the machine, and its toolbar carries the signed-in
# account's picture. Each shot is laid on a plate in the app's own dark grey,
# so nothing else on the desktop can appear at the edges either.
#
# The coordinates suit a large display; on a smaller one, check the result.
# Needs the release binary built first.

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$pad = [System.IO.Path]::GetTempPath()
$exe = Join-Path $root 'target\x86_64-pc-windows-msvc\release\bubbleTranslate.exe'
$out = Join-Path $PSScriptRoot 'store-shots'
if (-not (Test-Path $exe)) { throw "build the release binary first: cargo build --release --target x86_64-pc-windows-msvc" }
New-Item -ItemType Directory -Force -Path $out | Out-Null

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int t, bool repaint);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, IntPtr e);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int size);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  // GetWindowRect includes the invisible resize/shadow margin, so a crop to it
  // catches whatever is on the desktop behind. The DWM frame bounds are what
  // is actually drawn.
  public static RECT Visible(IntPtr h) {
    RECT r;
    if (DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out r);
    return r;
  }
  public static void Click(int x, int y) {
    SetCursorPos(x, y); System.Threading.Thread.Sleep(200);
    mouse_event(0x0002, 0, 0, 0, IntPtr.Zero); mouse_event(0x0004, 0, 0, 0, IntPtr.Zero);
  }
}
'@
$null = [W]::SetProcessDPIAware()

function Grab-Screen {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.X, $b.Y, 0, 0, $bmp.Size)
    $g.Dispose()
    return $bmp
}

# Crops a region and lays it on a 1920x1080 plate in the app's own dark grey,
# so the listing never shows whatever else happens to be on this desktop.
function Save-Shot($source, [int]$x, [int]$y, [int]$w, [int]$h, [string]$name, [int]$pad = 48) {
    $canvas = New-Object System.Drawing.Bitmap(1920, 1080)
    $g = [System.Drawing.Graphics]::FromImage($canvas)
    $g.Clear([System.Drawing.ColorTranslator]::FromHtml('#16171a'))
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    $maxW = 1920 - $pad * 2
    $maxH = 1080 - $pad * 2
    $scale = [math]::Min($maxW / [double]$w, $maxH / [double]$h)
    $dw = [int]($w * $scale); $dh = [int]($h * $scale)
    $dx = [int]((1920 - $dw) / 2); $dy = [int]((1080 - $dh) / 2)

    $src = New-Object System.Drawing.Rectangle($x, $y, $w, $h)
    $dst = New-Object System.Drawing.Rectangle($dx, $dy, $dw, $dh)
    $g.DrawImage($source, $dst, $src, [System.Drawing.GraphicsUnit]::Pixel)
    $g.Dispose()
    $path = Join-Path $out $name
    $canvas.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $canvas.Dispose()
    "  $name"
}

Get-Process bubbleTranslate, notepad -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 800

$sample = Join-Path $pad 'shot-sample.txt'
@'
Der Herbst kam frueh in diesem Jahr.

Die Blaetter faerbten sich golden, und jeden Morgen lag Nebel ueber
dem Fluss. Wer die Sprache nicht kennt, liest hier nur Buchstaben.
'@ | Set-Content -Path $sample -Encoding utf8

$env:BUBBLETRANSLATE_DEBUG = '1'
$app = Start-Process -FilePath $exe -PassThru `
    -RedirectStandardError (Join-Path $pad 'shot-trace.txt') `
    -RedirectStandardOutput (Join-Path $pad 'shot-out.txt')
Start-Sleep -Seconds 8

# --- 1: the window ----------------------------------------------------------
$main = (Get-Process -Id $app.Id).MainWindowHandle
if ($main -ne [IntPtr]::Zero) {
    [W]::MoveWindow($main, 300, 150, 1100, 1500, $true) | Out-Null
    Start-Sleep -Milliseconds 900
    [W]::SetForegroundWindow($main) | Out-Null
    Start-Sleep -Milliseconds 700
    $r = [W]::Visible($main)
    $shot = Grab-Screen
    Save-Shot $shot $r.Left $r.Top ($r.Right - $r.Left) ($r.Bottom - $r.Top) 'window.png'
    $shot.Dispose()
    # Out of the way for the next shot; on Windows the tray keeps the app alive.
    [W]::SendMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Start-Sleep -Seconds 2
}

# --- 2: the bubble over real text -------------------------------------------
$null = Start-Process notepad -ArgumentList $sample -PassThru
# Windows 11's Notepad is a packaged app: the process Start-Process hands back
# is often not the one that owns the window, so the handle is looked up by
# polling every notepad process rather than taken from that object.
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 30; $i++) {
    $cand = Get-Process notepad -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($cand) { $h = $cand.MainWindowHandle; break }
    Start-Sleep -Milliseconds 500
}
if ($h -eq [IntPtr]::Zero) { throw "notepad never showed a window" }

[W]::MoveWindow($h, 240, 180, 2600, 1240, $true) | Out-Null
Start-Sleep -Milliseconds 900
[W]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800

$r = [W]::Visible($h)
# A click inside the text puts the caret in the edit control; AppActivate alone
# leaves the accessibility read with nothing to find. It lands on the line that
# is about to be selected, because the bubble opens at the pointer -- clicking
# far below the text would leave a lake of empty page between the two.
[W]::Click(($r.Left + 620), ($r.Top + 300))
Start-Sleep -Milliseconds 700

$ws = New-Object -ComObject WScript.Shell
$ws.SendKeys('^{HOME}')
Start-Sleep -Milliseconds 500
$ws.SendKeys('{DOWN}{DOWN}')
Start-Sleep -Milliseconds 400
$ws.SendKeys('+{END}')
Start-Sleep -Milliseconds 2600

$shot = Grab-Screen
# Below the tab strip and the toolbar: the tabs carry the names of whatever
# files happen to be open on this machine, and the toolbar carries the signed-in
# account's picture. Neither belongs in a public listing. What is left is the
# text, the selection and the bubble, which is the whole point of the shot.
$cropX = $r.Left + 10
$cropY = $r.Top + 170
$cropW = 1600
$cropH = 900
Save-Shot $shot $cropX $cropY $cropW $cropH 'bubble.png'
$shot.Dispose()

Write-Host "---- trace ----"
Get-Content (Join-Path $pad 'shot-trace.txt') | Select-Object -Last 8

