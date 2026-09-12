# Builds bubbleTranslate.exe — the Windows download.
#
# One file, and nothing to install: the runtime is linked in statically (see
# .cargo\config.toml), the icon and version stamp are linked in by build.rs,
# and the settings live in %APPDATA% once the app first runs. Copying the .exe
# anywhere and double-clicking it is the whole installation.
#
#   powershell -ExecutionPolicy Bypass -File release.ps1
#
# Windows has no fat binary, so "universal" here means x86-64: it runs natively
# on every x64 machine and under the built-in emulation on an ARM64 one, which
# an ARM64-only build would not do in reverse.
#
# Unsigned, like the macOS build. SmartScreen therefore warns on the first
# launch on a machine that has not seen the file before, and the user clicks
# through "More info" › "Run anyway" once. To sign it, put a code-signing
# certificate in the machine's store and set:
#
#   $env:SIGN_THUMBPRINT = "<certificate thumbprint>"
#   powershell -ExecutionPolicy Bypass -File release.ps1

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$TARGET = 'x86_64-pc-windows-msvc'
$OUT = Join-Path $PSScriptRoot 'bubbleTranslate.exe'

# The toolchain that builds this target, which on an ARM64 machine is not the
# default one: cross-linking x64 from an ARM64 host needs a matching MSVC, and
# running the x64 toolchain under emulation is the shorter road.
$toolchain = ''
if ((rustc -vV | Select-String '^host:') -notmatch 'x86_64') {
    $installed = rustup toolchain list
    if ($installed -match 'x86_64-pc-windows-msvc') {
        $toolchain = '+stable-x86_64-pc-windows-msvc'
        Write-Host "host is not x64; building with stable-x86_64-pc-windows-msvc"
    }
}

# A stale copy of the icon would ship silently, so it is drawn every time. It
# is deterministic: an unchanged mark rewrites an identical file.
& (Join-Path $PSScriptRoot 'windows\make-icon.ps1')
Remove-Item (Join-Path $PSScriptRoot 'windows\preview-*.png') -ErrorAction SilentlyContinue

if ($toolchain) { cargo $toolchain build --release --target $TARGET }
else { cargo build --release --target $TARGET }
if ($LASTEXITCODE -ne 0) { throw "the build failed" }

$built = Join-Path $PSScriptRoot "target\$TARGET\release\bubbleTranslate.exe"
Copy-Item $built $OUT -Force

if ($env:SIGN_THUMBPRINT) {
    # Timestamped, so the signature outlives the certificate.
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName | Select-Object -Last 1
    if (-not $signtool) { throw "SIGN_THUMBPRINT is set but signtool.exe was not found" }
    & $signtool.FullName sign /sha1 $env:SIGN_THUMBPRINT /fd sha256 `
        /tr http://timestamp.digicert.com /td sha256 $OUT
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
    Write-Host "signed with certificate $env:SIGN_THUMBPRINT"
} else {
    Write-Host "not signed - SmartScreen will warn on the first launch."
    Write-Host "See this script's header for the signed path."
}

$size = [math]::Round((Get-Item $OUT).Length / 1MB, 1)
Write-Host ""
Write-Host "built $OUT ($size MB)"
Write-Host ""
Write-Host "First run:"
Write-Host "  double-click bubbleTranslate.exe"
Write-Host "  SmartScreen: More info > Run anyway (once, because it is unsigned)"
Write-Host "  then hold Shift and select text in any application."
