#!/bin/bash
# Draws macos/bubbleTranslate.icns from linux/bubbleTranslate.svg.
#
# The icon is checked in, so this only has to run when the mark changes. It is
# what System Settings shows beside bubbleTranslate under Accessibility and
# Screen Recording -- the two places a Mac user is asked to find the app by
# sight -- and up to 0.4.10 there was none, so both lists showed a blank
# placeholder there.
#
# The SVG is rendered by AppKit itself, so nothing needs installing.
#
#   macos/make-icon.sh

set -euo pipefail
cd "$(dirname "$0")/.."

SVG="linux/bubbleTranslate.svg"
OUT="macos/bubbleTranslate.icns"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SET="$WORK/bubbleTranslate.iconset"
mkdir -p "$SET"

cat > "$WORK/render.swift" <<'SWIFT'
import AppKit
let a = CommandLine.arguments
guard let img = NSImage(contentsOfFile: a[1]) else { fatalError("cannot read \(a[1])") }
let px = Int(a[3])!
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px,
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSGraphicsContext.current?.imageInterpolation = .high
img.draw(in: NSRect(x: 0, y: 0, width: px, height: px))
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: a[2]))
SWIFT
swiftc -O -o "$WORK/render" "$WORK/render.swift"

for size in 16 32 128 256 512; do
    "$WORK/render" "$SVG" "$SET/icon_${size}x${size}.png" "$size"
    "$WORK/render" "$SVG" "$SET/icon_${size}x${size}@2x.png" "$((size * 2))"
done
iconutil -c icns -o "$OUT" "$SET"
echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
