<#
.SYNOPSIS
  Builds Moonrise for Windows: fetches the pinned LGPL libmpv, publishes the app self-contained,
  and makes the unsigned installer. Needs the .NET SDK and Inno Setup (winget install
  Microsoft.DotNet.SDK.10 JRSoftware.InnoSetup). See docs/dev/windows.md.

.PARAMETER Dev
  The Dev app (Moonrise Dev: its own name, icon and data folder), and no installer. It's the
  build to test with, so testing never touches the app someone watches with.

.PARAMETER SkipInstaller
  Stop after publishing (build\publish).

.PARAMETER LibMpvArchive
  The pinned libmpv archive (the file named in $LibMpvFile) from a local path, instead of a
  download. It must match the pinned SHA-256; it is copied into the cache.

.PARAMETER CacheDir
  Where the libmpv archive is cached and unpacked. Default: %LOCALAPPDATA%\moonrise-build. An
  archive already there is used without a download.
#>
param([switch]$Dev, [switch]$SkipInstaller, [string]$LibMpvArchive, [string]$CacheDir)
$ErrorActionPreference = 'Stop'

# The LGPL build of libmpv (mpv and FFmpeg without GPL parts), see docs/adr/0014-windows-app.md.
# The project keeps its own copy of the pinned file as a release asset (tag deps-libmpv-<tag>),
# because the upstream keeps its builds for 30 days; the upstream is only the fallback. To re-pin,
# follow "Updating libmpv" in docs/dev/windows.md and change these values together.
$ProjectRepo = 'paralainer/moonrise'
$LibMpvTag = '2026-11-09-a61f6c3abd'
$LibMpvFile = 'mpv-dev-lgpl-x86_64-20261109-git-a61f6c3abd.7z'
$LibMpvSha256 = 'a981f848c79859a8bb48c24f817b80784dbf0d06d4bd24336b9bd6a06ac05d9c'
$LibMpvUrls = @(
    "https://github.com/$ProjectRepo/releases/download/deps-libmpv-$LibMpvTag/$LibMpvFile",
    "https://github.com/zhongfly/mpv-winbuild/releases/download/$LibMpvTag/$LibMpvFile")

$Root = $PSScriptRoot
$Cache = if ($CacheDir) { $CacheDir } else { Join-Path $env:LOCALAPPDATA 'moonrise-build' }
$Out = Join-Path $Root 'build'
New-Item -ItemType Directory -Force $Cache, $Out | Out-Null

function Get-Sha256($path) { (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant() }

function Get-LibMpvArchive {
    $archive = Join-Path $Cache $LibMpvFile
    if ($LibMpvArchive) {
        if (!(Test-Path -PathType Leaf $LibMpvArchive)) { throw "No file at -LibMpvArchive $LibMpvArchive" }
        $hash = Get-Sha256 $LibMpvArchive
        if ($hash -ne $LibMpvSha256) {
            throw "Refusing $LibMpvArchive`: its SHA-256 is $hash, the pinned $LibMpvFile has $LibMpvSha256."
        }
        if ((Resolve-Path $LibMpvArchive).Path -ne [IO.Path]::GetFullPath($archive)) {
            Copy-Item -Force $LibMpvArchive $archive
        }
        return $archive
    }
    if (Test-Path $archive) {
        $hash = Get-Sha256 $archive
        if ($hash -ne $LibMpvSha256) {
            throw "Refusing the cached $archive`: its SHA-256 is $hash, the pin is $LibMpvSha256. Delete it and run again."
        }
        return $archive
    }
    $failed = @()
    foreach ($url in $LibMpvUrls) {
        Write-Host "Downloading $url"
        & curl.exe -sSLf -o "$archive.tmp" $url
        if ($LASTEXITCODE -ne 0) {
            $failed += "  $url (download failed)"
        } elseif ((Get-Sha256 "$archive.tmp") -ne $LibMpvSha256) {
            $failed += "  $url (wrong SHA-256, not used)"
        } else {
            Move-Item -Force "$archive.tmp" $archive
            return $archive
        }
        Remove-Item -Force -ErrorAction SilentlyContinue "$archive.tmp"
    }
    throw ("Could not get libmpv ($LibMpvFile, SHA-256 $LibMpvSha256) from:`n" + ($failed -join "`n") +
        "`nWhile the project repository is private its release needs a sign-in, which this script " +
        "doesn't use. Fetch the file on a signed-in machine (gh release download " +
        "deps-libmpv-$LibMpvTag -R $ProjectRepo), then pass it with -LibMpvArchive <path> or put " +
        "it in $Cache. See docs/dev/windows.md.")
}

function Get-LibMpv {
    $archive = Get-LibMpvArchive
    $dir = Join-Path $Cache 'libmpv-unpacked'
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    New-Item -ItemType Directory $dir | Out-Null
    & tar.exe -xf $archive -C $dir
    if ($LASTEXITCODE -ne 0) { throw 'Could not unpack libmpv (Windows 10 1803+ tar.exe handles .7z).' }
    return (Join-Path $dir 'libmpv-2.dll')
}

function Find-Iscc {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 6\ISCC.exe')
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }
    throw 'Inno Setup not found: winget install JRSoftware.InnoSetup'
}

$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$libmpv = Get-LibMpv
$publishName = if ($Dev) { 'publish-dev' } else { 'publish' }
$publish = Join-Path $Out $publishName
if (Test-Path $publish) { Remove-Item -Recurse -Force $publish }

$props = @('-c', 'Release', '-r', 'win-x64', '--self-contained', 'true', '-o', $publish, '-p:Platform=x64')
if ($Dev) { $props += '-p:MoonriseDev=true' }
Write-Host "Publishing $(if ($Dev) { 'Moonrise Dev' } else { 'Moonrise' })"
& dotnet publish (Join-Path $Root 'Moonrise.App\Moonrise.App.csproj') @props
if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed' }

Copy-Item -Force $libmpv $publish
Copy-Item -Force (Join-Path $Root 'THIRD-PARTY-NOTICES.txt') $publish

$version = ([xml](Get-Content (Join-Path $Root 'Moonrise.App\Moonrise.App.csproj'))).Project.PropertyGroup.Version | Select-Object -First 1
if ($Dev) { Write-Host "Moonrise Dev published to $publish"; exit 0 }

if (!$SkipInstaller) {
    $iscc = Find-Iscc
    $installerOut = Join-Path $Out 'installer'
    New-Item -ItemType Directory -Force $installerOut | Out-Null
    & $iscc "/DAppVersion=$version" "/DPublishDir=$publish" "/DOutputDir=$installerOut" (Join-Path $Root 'installer\moonrise.iss')
    if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }
    Write-Host "Installer: $installerOut\Moonrise-Setup-$version.exe"
}
