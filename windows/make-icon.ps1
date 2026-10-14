# Draws windows\bubbleTranslate.ico from the same mark as linux\bubbleTranslate.svg.
#
# The icon is checked in, so this only has to run when the mark changes. It is
# here rather than in a designer's hands because the mark is six shapes, and
# because the small sizes are not the large one shrunk: at 16 pixels the speech
# bubble around the globe becomes four grey smudges, so the globe is drawn on
# its own and drawn heavier.
#
# Every entry is a PNG, which Windows has understood inside an .ico since
# Vista and which keeps the whole file under 30 KB with the 256-pixel size in
# it.
#
#   powershell -ExecutionPolicy Bypass -File windows\make-icon.ps1

Add-Type -AssemblyName System.Drawing

$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot 'bubbleTranslate.ico'

# The mark, in the same 128-unit space the SVG uses.
$INK      = [System.Drawing.ColorTranslator]::FromHtml('#e8e8ea')  # the globe
$BUBBLE   = [System.Drawing.ColorTranslator]::FromHtml('#1e1f22')  # its card
$EDGE     = [System.Drawing.ColorTranslator]::FromHtml('#5b5f66')  # the outline

function New-Canvas([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.ScaleTransform($size / 128.0, $size / 128.0)
    return @($bmp, $g)
}

function Add-RoundedRect($path, [float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $d = $r * 2
    $path.AddArc($x, $y, $d, $d, 180, 90)
    $path.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $path.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $path.CloseFigure()
}

# The globe: an outline, a meridian, and the parallels. Drawn around a centre
# so the same routine serves the full mark and the small-size one.
function Draw-Globe($g, [float]$cx, [float]$cy, [float]$r, [float]$width, $color) {
    $pen = New-Object System.Drawing.Pen($color, $width)
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawEllipse($pen, $cx - $r, $cy - $r, $r * 2, $r * 2)
    $g.DrawEllipse($pen, $cx - $r * 0.42, $cy - $r, $r * 0.84, $r * 2)
    $g.DrawLine($pen, $cx - $r, $cy, $cx + $r, $cy)
    $g.DrawLine($pen, $cx - $r * 0.85, $cy - $r * 0.54, $cx + $r * 0.85, $cy - $r * 0.54)
    $g.DrawLine($pen, $cx - $r * 0.85, $cy + $r * 0.54, $cx + $r * 0.85, $cy + $r * 0.54)
    $pen.Dispose()
}

# The full mark: a globe inside a speech bubble.
function Draw-Mark($g) {
    $fill = New-Object System.Drawing.SolidBrush($BUBBLE)
    $edge = New-Object System.Drawing.Pen($EDGE, 4)
    $edge.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round

    # The tail's top edge lies exactly along the card's bottom edge, so the two
    # outlines overlap there and no seam is ever visible — the same trick the
    # SVG uses.
    $tail = New-Object System.Drawing.Drawing2D.GraphicsPath
    $tail.AddPolygon(@(
        (New-Object System.Drawing.PointF(40, 96)),
        (New-Object System.Drawing.PointF(40, 116)),
        (New-Object System.Drawing.PointF(62, 96))
    ))
    $g.FillPath($fill, $tail)
    $g.DrawPath($edge, $tail)

    $card = New-Object System.Drawing.Drawing2D.GraphicsPath
    Add-RoundedRect $card 8 20 112 76 18
    $g.FillPath($fill, $card)
    $g.DrawPath($edge, $card)

    Draw-Globe $g 64 58 26 4 $INK
}

# Small sizes: the same bubble, redrawn rather than shrunk.
#
# Shrinking the full mark is what fails at 16 pixels, not the bubble itself.
# The card there carries margins it cannot afford, which leaves the globe
# inside it about five pixels across. So the card is pushed out to the edges of
# the canvas, the tail is widened until it survives the grid, and the globe
# keeps only the three lines that still read at that size — its outline, its
# meridian and its equator. The two parallels of the full mark close up into a
# smudge long before the rest of it does.
function Draw-Small($g) {
    $fill = New-Object System.Drawing.SolidBrush($BUBBLE)
    $edge = New-Object System.Drawing.Pen($EDGE, 7)
    $edge.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round

    # Tail first, and its top edge along the card's bottom edge, so the two
    # outlines overlap there rather than meeting in a seam.
    $tail = New-Object System.Drawing.Drawing2D.GraphicsPath
    $tail.AddPolygon(@(
        (New-Object System.Drawing.PointF(30, 90)),
        (New-Object System.Drawing.PointF(30, 124)),
        (New-Object System.Drawing.PointF(66, 90))
    ))
    $g.FillPath($fill, $tail)
    $g.DrawPath($edge, $tail)

    $card = New-Object System.Drawing.Drawing2D.GraphicsPath
    Add-RoundedRect $card 4 6 120 84 20
    $g.FillPath($fill, $card)
    $g.DrawPath($edge, $card)

    $pen = New-Object System.Drawing.Pen($INK, 9)
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $cx = 64.0; $cy = 49.0; $r = 29.0
    $g.DrawEllipse($pen, $cx - $r, $cy - $r, $r * 2, $r * 2)
    $g.DrawEllipse($pen, $cx - $r * 0.42, $cy - $r, $r * 0.84, $r * 2)
    $g.DrawLine($pen, $cx - $r, $cy, $cx + $r, $cy)
    $pen.Dispose()
    $edge.Dispose()
    $fill.Dispose()
}

$entries = @()
foreach ($size in 256, 128, 64, 48, 32, 24, 20, 16) {
    $canvas = New-Canvas $size
    $bmp, $g = $canvas
    # The card is worth drawing only where it is legible. In the taskbar and
    # the notification area — 32 pixels and under — it reduces to a dark smudge
    # that swallows the globe inside it, and the globe is the part that says
    # what the app does.
    if ($size -ge 48) { Draw-Mark $g } else { Draw-Small $g }
    $g.Dispose()

    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $entries += , @{ Size = $size; Bytes = $stream.ToArray() }
    # Handy for looking at the result; the .ico is what ships.
    [System.IO.File]::WriteAllBytes((Join-Path $PSScriptRoot "preview-$size.png"), $stream.ToArray())
}

# The .ico container: a six-byte header, one sixteen-byte directory entry per
# image, then the images themselves.
$file = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($file)
$w.Write([uint16]0)                  # reserved
$w.Write([uint16]1)                  # type: icon
$w.Write([uint16]$entries.Count)
$offset = 6 + 16 * $entries.Count
foreach ($entry in $entries) {
    # 256 is written as zero, which is how the format says "two hundred and
    # fifty-six" in one byte.
    $w.Write([byte]($entry.Size % 256))
    $w.Write([byte]($entry.Size % 256))
    $w.Write([byte]0)                # palette colours: none, it is true colour
    $w.Write([byte]0)                # reserved
    $w.Write([uint16]1)              # colour planes
    $w.Write([uint16]32)             # bits per pixel
    $w.Write([uint32]$entry.Bytes.Length)
    $w.Write([uint32]$offset)
    $offset += $entry.Bytes.Length
}
foreach ($entry in $entries) { $w.Write($entry.Bytes) }
$w.Flush()
[System.IO.File]::WriteAllBytes($out, $file.ToArray())
$w.Dispose()

"wrote $out ($([math]::Round((Get-Item $out).Length / 1KB, 1)) KB, $($entries.Count) sizes)"
