# Builds bubbleTranslate.msix — the Microsoft Store package.
#
#   powershell -ExecutionPolicy Bypass -File package.ps1
#
# This is the Store channel and it is kept apart from the download on purpose.
# What it does NOT do is as much of the point as what it does:
#
#   * it does not touch latest.json. That file is the download channel's, and
#     every installed copy reads it on startup. A Store version written into
#     it would tell those copies to update and hand them a URL for an asset
#     that does not exist.
#   * it does not create a GitHub release, upload an asset, or run
#     fill-release.sh.
#   * it does not rename the "Unreleased" heading in CHANGELOG.md.
#
# The .msix it writes goes to Partner Center by hand, and nowhere else. The
# two channels therefore sit at different versions whenever certification is
# slower than a release, which is normal rather than a problem: a Store copy
# never reads latest.json, because the build has no update check at all.
#
# Signing: the Store re-signs on ingestion, so the package uploaded there does
# not need a certificate. Pass -SelfSign to get one signed with a throwaway
# local certificate, which is the only way to install it on this machine to
# test it. Never upload a self-signed package.

param(
    [switch]$SelfSign
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# --- who this package says it is --------------------------------------------
#
# The three identity fields have to match the product in Partner Center
# exactly or the upload is rejected, and they are not guessable -- they come
# from the product's Identity page. Kept in a file rather than in the manifest
# so the manifest stays a template and nobody edits it per release.

$identityPath = Join-Path $PSScriptRoot 'windows\store-identity.json'
if (-not (Test-Path $identityPath)) {
    Write-Host "error: windows\store-identity.json is missing." -ForegroundColor Red
    Write-Host ""
    Write-Host "       It carries the three fields Partner Center assigns, which must"
    Write-Host "       match the reserved product exactly. Find them under the product's"
    Write-Host "       Product management > Product identity page, and write:"
    Write-Host ""
    Write-Host '         {'
    Write-Host '           "identityName": "12345Publisher.bubbleTranslate",'
    Write-Host '           "publisher": "CN=ABCD1234-0000-0000-0000-00000000ABCD",'
    Write-Host '           "publisherDisplayName": "Your publisher display name"'
    Write-Host '         }'
    Write-Host ""
    Write-Host "       publisher is the full CN= string, not the display name."
    exit 1
}

$identity = Get-Content $identityPath -Raw | ConvertFrom-Json
foreach ($field in 'identityName', 'publisher', 'publisherDisplayName') {
    if (-not $identity.$field) { throw "store-identity.json has no $field" }
}

# --- the version ------------------------------------------------------------
#
# Cargo.toml is the one place a version lives, the same as for the download.
# A package version is four parts and the Store requires the last to be zero;
# it is reserved for Microsoft's own use when it repackages.

$version = (Select-String -Path (Join-Path $PSScriptRoot 'Cargo.toml') -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw "could not read a version from Cargo.toml" }
$packageVersion = "$version.0"

# Refused rather than warned, the same as release.ps1: the Store listing's
# release notes are this section, and a submission whose notes are written
# afterwards is written from memory.
$changelog = Get-Content (Join-Path $PSScriptRoot 'CHANGELOG.md') -Raw
if ($changelog -notmatch "(?m)^## $([regex]::Escape($version))(\s|$)") {
    Write-Host "error: CHANGELOG.md has no section for $version." -ForegroundColor Red
    exit 1
}

# --- the build --------------------------------------------------------------

$TARGET = 'x86_64-pc-windows-msvc'

# Cross-linking x64 from an ARM64 host needs the matching MSVC; release.ps1
# picks the same toolchain the same way.
$toolchain = ''
if ((rustc -vV | Select-String '^host:') -notmatch 'x86_64') {
    if ((rustup toolchain list) -match 'x86_64-pc-windows-msvc') {
        $toolchain = '+stable-x86_64-pc-windows-msvc'
        Write-Host "host is not x64; building with stable-x86_64-pc-windows-msvc"
    }
}

& (Join-Path $PSScriptRoot 'windows\make-icon.ps1')
Remove-Item (Join-Path $PSScriptRoot 'windows\preview-*.png') -ErrorAction SilentlyContinue

if ($toolchain) { cargo $toolchain build --release --target $TARGET --features store }
else { cargo build --release --target $TARGET --features store }
if ($LASTEXITCODE -ne 0) { throw "the build failed" }

$built = Join-Path $PSScriptRoot "target\$TARGET\release\bubbleTranslate.exe"

# The mirror of release.ps1's check, and worth as much: a package built
# without the feature would carry the updater, which cannot replace a file
# inside a package and would fail in front of a user -- and ships an app that
# updates itself outside the Store, which the policy does not allow.
$stamp = Join-Path $env:TEMP "bubbleTranslate-package-$PID.txt"
Start-Process -FilePath $built -ArgumentList '--version' -Wait -WindowStyle Hidden `
    -RedirectStandardOutput $stamp | Out-Null
$said = Get-Content $stamp -Raw -ErrorAction SilentlyContinue
Remove-Item $stamp -Force -ErrorAction SilentlyContinue
if ($said -notmatch 'channel\s+store') {
    Write-Host "error: the binary built is not the Store build." -ForegroundColor Red
    Write-Host "       It still has the update check, which a packaged app may not use."
    exit 1
}
Write-Host "built the Store binary ($version), update check off"

# --- the package ------------------------------------------------------------

& (Join-Path $PSScriptRoot 'windows\make-store-assets.ps1')

$stage = Join-Path $PSScriptRoot 'target\msix'
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'Assets') | Out-Null

Copy-Item $built (Join-Path $stage 'bubbleTranslate.exe') -Force
Copy-Item (Join-Path $PSScriptRoot 'windows\store-assets\*.png') (Join-Path $stage 'Assets') -Force
# The listing logo is uploaded to Partner Center, not carried in the package.
Remove-Item (Join-Path $stage 'Assets\StoreListing-300x300.png') -Force -ErrorAction SilentlyContinue

$manifest = Get-Content (Join-Path $PSScriptRoot 'windows\AppxManifest.xml') -Raw
$manifest = $manifest.Replace('@IDENTITY_NAME@', $identity.identityName).
                      Replace('@PUBLISHER@', $identity.publisher).
                      Replace('@PUBLISHER_DISPLAY_NAME@', $identity.publisherDisplayName).
                      Replace('@VERSION@', $packageVersion)
Set-Content -Path (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding utf8

# makeappx and signtool live in the Windows SDK, under a versioned folder.
function Find-SdkTool([string]$name) {
    $found = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\$name" -ErrorAction SilentlyContinue |
        Sort-Object FullName | Select-Object -Last 1
    if (-not $found) {
        throw "$name was not found. Install the Windows SDK (it ships with Visual Studio's 'Desktop development with C++' workload)."
    }
    return $found.FullName
}

$makeappx = Find-SdkTool 'makeappx.exe'
$MSIX = Join-Path $PSScriptRoot 'bubbleTranslate.msix'
Remove-Item $MSIX -Force -ErrorAction SilentlyContinue
& $makeappx pack /d $stage /p $MSIX /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed" }

if ($SelfSign) {
    # Local testing only. The subject must equal the manifest's Publisher or
    # Windows refuses to install the package.
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $identity.publisher } | Select-Object -First 1
    if (-not $cert) {
        Write-Host "making a throwaway certificate for $($identity.publisher)"
        $cert = New-SelfSignedCertificate -Type Custom -Subject $identity.publisher `
            -KeyUsage DigitalSignature -FriendlyName "bubbleTranslate local test" `
            -CertStoreLocation "Cert:\CurrentUser\My" `
            -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
        Write-Host "to install the package this machine must trust it:" -ForegroundColor Yellow
        Write-Host "  export it and import into Local Machine > Trusted People" -ForegroundColor Yellow
    }
    $signtool = Find-SdkTool 'signtool.exe'
    & $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $MSIX
    if ($LASTEXITCODE -ne 0) { throw "signing failed" }
    Write-Host "signed for local testing - do NOT upload this file"
}

$size = [math]::Round((Get-Item $MSIX).Length / 1MB, 1)
Write-Host ""
Write-Host "built $MSIX ($size MB), package version $packageVersion"
Write-Host ""
Write-Host "Nothing was published. To submit:"
Write-Host "  Partner Center > bubbleTranslate > Packages > upload the .msix"
Write-Host "  release notes: the '## $version' section of CHANGELOG.md"
Write-Host "  listing logo:  windows\store-assets\StoreListing-300x300.png"
Write-Host ""
Write-Host "latest.json and the downloads repository were not touched, which is"
Write-Host "what keeps the Store channel from disturbing the download."
