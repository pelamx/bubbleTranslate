# Draws the Microsoft Store package's tile images, from the same mark as the
# .ico and the SVG.
#
#   powershell -ExecutionPolicy Bypass -File windows\make-store-assets.ps1
#
# `package.ps1` runs this, so it needs running by hand only to look at the
# result. Like make-icon.ps1 it is deterministic: an unchanged mark rewrites
# identical files.
#
# The drawing itself is make-icon.ps1's, dot-sourced rather than copied --
# there is one mark, and a second copy of it would drift. Dot-sourcing also
# rewrites the .ico, which is harmless because it rewrites it identically.
#
# Each logo is drawn at its real size rather than scaled down from one big
# one, and which mark is drawn follows make-icon.ps1's rule: the speech bubble
# only where it is legible, the globe alone below that. The threshold matters
# here because the app-list icon is 44 pixels at 100% scale -- under it -- and
# 88 at 200%, over it. That is correct rather than inconsistent: the small one
# is small because the screen is showing it small.

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

# Brings in New-Canvas, Draw-Mark, Draw-Small and the palette.
. (Join-Path $PSScriptRoot 'make-icon.ps1') | Out-Null
Remove-Item (Join-Path $PSScriptRoot 'preview-*.png') -ErrorAction SilentlyContinue

$out = Join-Path $PSScriptRoot 'store-assets'
New-Item -ItemType Directory -Force -Path $out | Out-Null

# The wide tile is the one shape that is not square, so the mark cannot simply
# be scaled to the canvas: it is drawn at the tile's height and centred, with
# the extra width left transparent either side. Windows requires a wide logo
# as soon as a 310x310 one is offered, which is why it exists at all.
function Write-WideLogo([string]$name, [int]$width, [int]$height) {
    $bmp = New-Object System.Drawing.Bitmap($width, $height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.TranslateTransform([float](($width - $height) / 2.0), 0.0)
    $g.ScaleTransform($height / 128.0, $height / 128.0)
    Draw-Mark $g
    $g.Dispose()
    $path = Join-Path $out $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $path
}

function Write-Logo([string]$name, [int]$size) {
    $canvas = New-Canvas $size
    $bmp, $g = $canvas
    if ($size -ge 48) { Draw-Mark $g } else { Draw-Small $g }
    $g.Dispose()
    $path = Join-Path $out $name
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    return $path
}

# The three logos a package must carry, each at the scales Windows picks
# between on different displays. A missing scale is not an error -- Windows
# scales the nearest one -- but a scaled 44-pixel globe on a 200% display is
# visibly soft, and these cost nothing to draw.
$plan = @(
    @{ Base = 'Square44x44Logo';  Size = 44  },
    @{ Base = 'Square150x150Logo'; Size = 150 },
    @{ Base = 'Square71x71Logo';   Size = 71  },
    @{ Base = 'Square310x310Logo'; Size = 310 },
    @{ Base = 'StoreLogo';         Size = 50  }
)
$scales = 100, 125, 150, 200, 400

$count = 0
foreach ($logo in $plan) {
    # The unscaled name is what the manifest points at; Windows finds the
    # scaled ones beside it by name.
    $null = Write-Logo "$($logo.Base).png" $logo.Size
    $count++
    foreach ($scale in $scales) {
        $size = [int][math]::Round($logo.Size * $scale / 100.0)
        $null = Write-Logo "$($logo.Base).scale-$scale.png" $size
        $count++
    }
}

$null = Write-WideLogo 'Wide310x150Logo.png' 310 150
$count++
foreach ($scale in $scales) {
    $w = [int][math]::Round(310 * $scale / 100.0)
    $h = [int][math]::Round(150 * $scale / 100.0)
    $null = Write-WideLogo "Wide310x150Logo.scale-$scale.png" $w $h
    $count++
}

# The app-list icon also has "target size" variants, which Windows uses in
# places that want an exact pixel count: the taskbar, Alt+Tab, the Start list.
# Unplated means no square colour plate behind it, which suits a mark that is
# already a card on transparency.
foreach ($size in 16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256) {
    $null = Write-Logo "Square44x44Logo.targetsize-$size.png" $size
    $null = Write-Logo "Square44x44Logo.targetsize-$size`_altform-unplated.png" $size
    $count += 2
}

# Not part of the package: Partner Center asks for a 300x300 logo for the
# listing page itself, uploaded there rather than built in.
$null = Write-Logo 'StoreListing-300x300.png' 300
$count++

"wrote $count images to $out"
