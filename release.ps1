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

# --- the record -------------------------------------------------------------
#
# Refused rather than warned: a release whose entry is written afterwards is
# written from memory, and the people it is for are the two running the app.

$version = (Select-String -Path (Join-Path $PSScriptRoot 'Cargo.toml') -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
$changelog = Get-Content (Join-Path $PSScriptRoot 'CHANGELOG.md') -Raw
if ($changelog -notmatch "(?m)^## $([regex]::Escape($version))(\s|$)") {
    Write-Host "error: CHANGELOG.md has no section for $version." -ForegroundColor Red
    Write-Host "       Add one - rename '## Unreleased' to '## $version - $(Get-Date -Format yyyy-MM-dd)'"
    Write-Host "       and say, for the people using the app, what changed and why."
    exit 1
}

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

# Windows will not let a running program be overwritten, and the program most
# likely to be running while this builds is this one. It will, however, let a
# running program be *renamed*: the handle follows the file rather than the
# path. So the old download is moved aside, the new one takes its place, and
# the leftover is swept up on the next release, once whatever was holding it
# has exited.
$aside = "$OUT.locked-old"
if (Test-Path $aside) { Remove-Item $aside -Force -ErrorAction SilentlyContinue }
try {
    Copy-Item $built $OUT -Force -ErrorAction Stop
} catch {
    Write-Host "the old bubbleTranslate.exe is running; moving it aside"
    Move-Item $OUT $aside -Force
    Copy-Item $built $OUT -Force
}

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

# The zip is the download the README offers first, and it exists because of the
# signature this build does not have: a browser handed a bare unsigned .exe
# says it "isn't commonly downloaded" and discards it unless the user digs the
# file back out of the warning. The same bytes inside a zip arrive without the
# warning, and at 7 MB rather than 17. It is zipped after signing so that a
# signed .exe is what goes in, and uploaded beside the .exe -- latest.json
# points installed copies at the zip, which is what the updater's link opens.
$ZIP = Join-Path $PSScriptRoot 'bubbleTranslate-windows-x64.zip'
Remove-Item $ZIP -Force -ErrorAction SilentlyContinue
Compress-Archive -Path $OUT -DestinationPath $ZIP -CompressionLevel Optimal

# latest.json is what running copies read to say "a new version is available".
# It lives in the downloads repository -- the one that stays public -- rather
# than beside the source, so it is fetched, patched and put back through the
# API. Only the Windows line is touched: the other platforms are released on
# their own machines.
$repo = 'bubbleTranslate/downloads'
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "gh is not installed; latest.json was not published. Install it, or edit latest.json in $repo by hand."
}

$encoded = gh api "repos/$repo/contents/latest.json" --jq .content
$sha = gh api "repos/$repo/contents/latest.json" --jq .sha
$raw = [System.Text.Encoding]::UTF8.GetString([System.Convert]::FromBase64String(($encoded -replace '\s', '')))

# Edited in place rather than round-tripped through ConvertFrom-Json: the
# ConvertTo-Json of PowerShell 5.1 re-indents the whole file and aligns the
# colons, so every Windows release would arrive as a diff of all three
# platforms. A substitution touches the one value, the way release.sh does.
$pattern = '("windows"\s*:\s*\{\s*"version"\s*:\s*")([^"]*)(")'
$found = [regex]::Match($raw, $pattern)
if (-not $found.Success) { throw "latest.json in $repo has no windows version to update" }
if ($found.Groups[2].Value -eq $version) {
    Write-Host "warning: latest.json already says Windows $version - bump the version in"
    Write-Host "         Cargo.toml, or installed copies will not be told about this build"
}
$raw = [regex]::Replace($raw, $pattern, "`${1}$version`${3}")

# And the URL beside it, which names the release the download is an asset of.
# Bumping the version alone is how a build gets announced as new and then hands
# over the previous one: the app compares versions and opens whatever URL it is
# given, so the two have to move together.
$zipName = Split-Path $ZIP -Leaf
$url = "https://github.com/$repo/releases/download/v$version/$zipName"
$urlPattern = '("windows"\s*:\s*\{[\s\S]*?"url"\s*:\s*")[^"]*(")'
$raw = [regex]::Replace($raw, $urlPattern, "`${1}$url`${2}")

$content = [System.Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($raw))
gh api "repos/$repo/contents/latest.json" -X PUT -f message="windows $version" -f sha="$sha" -f content="$content" --jq .commit.sha | Out-Null
Write-Host "published: installed copies on Windows are now told about $version"

$size = [math]::Round((Get-Item $OUT).Length / 1MB, 1)
$zipSize = [math]::Round((Get-Item $ZIP).Length / 1MB, 1)
Write-Host ""
Write-Host "built $OUT ($size MB), version $version"
Write-Host "      $ZIP ($zipSize MB)"
Write-Host "upload both to the v$version release in $repo:"
Write-Host "  gh release upload v$version -R $repo $OUT $ZIP --clobber"
Write-Host "(latest.json is already published; neither file is tracked)"
Write-Host ""
Write-Host "First run:"
Write-Host "  double-click bubbleTranslate.exe"
Write-Host "  SmartScreen: More info > Run anyway (once, because it is unsigned)"
Write-Host "  then hold Shift and select text in any application."
