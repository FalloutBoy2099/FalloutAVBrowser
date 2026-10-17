#Requires -Version 5.1
<#
.SYNOPSIS
    Builds the release binaries and packs each one into publish\<name>-<version>-<os>-<arch>.zip

.DESCRIPTION
    Platforms: windows-x64, windows-arm64, linux-x64, linux-arm64, macos-x64, macos-arm64.

    How each target is built
      * Same OS as this machine (or Windows -> Windows, macOS -> macOS): plain `cargo build`.
      * Anything else: `cargo zigbuild` (zig is used as the cross linker).
          - Linux targets only need zig + cargo-zigbuild (glibc 2.17 is targeted, so the binaries run on old distributions).
          - macOS targets also need a macOS SDK (Apple licence: it cannot be downloaded by this script). Extract it somewhere
            and set SDKROOT, for example  $env:SDKROOT = 'C:\sdk\MacOSX14.sdk'.  Without it, build the macOS targets on a Mac.

    One-time setup on Windows
        winget install zig.zig
        cargo install cargo-zigbuild
        # Windows ARM64 also needs the Visual Studio component "MSVC v143 - VS 2022 C++ ARM64 build tools".

    Every package contains a folder <name>-<version>-<os>-<arch> with the executable and the README. Unix executables get
    the execute permission inside the zip, so `unzip` keeps them runnable.

.EXAMPLE
    .\publish.ps1                                   # everything that can be built
.EXAMPLE
    .\publish.ps1 -Platforms windows-x64,linux-x64  # a subset
.EXAMPLE
    .\publish.ps1 -Clean -KeepBuild                 # start from an empty publish folder, keep target\<triple> for faster re-runs
#>
[CmdletBinding()]
param(
    [ValidateSet('windows-x64', 'windows-arm64', 'linux-x64', 'linux-arm64', 'macos-x64', 'macos-arm64')]
    [string[]]$Platforms = @('windows-x64', 'windows-arm64', 'linux-x64', 'linux-arm64', 'macos-x64', 'macos-arm64'),
    [string]$OutDir = 'publish',
    [string]$BuildProfile = 'release',
    # Empty the output folder first.
    [switch]$Clean,
    # By default the temporary build folders of the cross targets (target\<triple>) are deleted once all builds are done.
    # This keeps them (faster re-runs, but several GB of disk space).
    [switch]$KeepBuild
)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# ------------------------------------------------------------------ project information
$manifest = Get-Content -Raw 'Cargo.toml'
$name = [regex]::Match($manifest, '(?m)^name\s*=\s*"([^"]+)"').Groups[1].Value
$version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $name -or -not $version) { throw 'Cannot read name/version from Cargo.toml' }
$profileDir = if ($BuildProfile -eq 'dev') { 'debug' } else { $BuildProfile }

$hostTriple = [regex]::Match((& rustc -vV | Out-String), '(?m)^host:\s*(\S+)').Groups[1].Value
# Windows PowerShell 5.1 (edition "Desktop") has no $IsWindows / $IsMacOS variables and only runs on Windows.
$hostOs = if ($PSVersionTable.PSEdition -eq 'Desktop' -or $IsWindows) { 'windows' } elseif ($IsMacOS) { 'macos' } else { 'linux' }
Add-Type -AssemblyName System.IO.Compression

$table = [ordered]@{
    'windows-x64'   = @{ Triple = 'x86_64-pc-windows-msvc';    Os = 'windows'; Exe = ".exe" }
    'windows-arm64' = @{ Triple = 'aarch64-pc-windows-msvc';   Os = 'windows'; Exe = ".exe" }
    'linux-x64'     = @{ Triple = 'x86_64-unknown-linux-gnu';  Os = 'linux';   Exe = '' }
    'linux-arm64'   = @{ Triple = 'aarch64-unknown-linux-gnu'; Os = 'linux';   Exe = '' }
    'macos-x64'     = @{ Triple = 'x86_64-apple-darwin';       Os = 'macos';   Exe = '' }
    'macos-arm64'   = @{ Triple = 'aarch64-apple-darwin';      Os = 'macos';   Exe = '' }
}

# ------------------------------------------------------------------ helpers
function Test-Command([string]$cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

function Invoke-Native {
    param([string]$What, [scriptblock]$Command)
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)" }
}

function Install-RustTarget([string]$triple) {
    $installed = (& rustup target list --installed) -split "`n" | ForEach-Object { $_.Trim() }
    if ($installed -notcontains $triple) {
        Write-Host "  rustup target add $triple"
        Invoke-Native "rustup target add $triple" { & rustup target add $triple }
    }
}

# Windows -> Windows on another architecture needs the matching MSVC cross tools; without them rustc silently falls back
# to whatever `link` is on the PATH (for example the Unix one from Git Bash) and fails after minutes of compiling.
function Test-MsvcCross([string]$triple) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return }
    $vs = & $vswhere -latest -products * -property installationPath
    if (-not $vs) { return }
    $hostArch = if ($hostTriple -like 'aarch64*') { 'arm64' } else { 'x64' }
    $targetArch = if ($triple -like 'aarch64*') { 'arm64' } else { 'x64' }
    $tools = Join-Path $vs 'VC\Tools\MSVC'
    $found = Get-ChildItem $tools -Directory -ErrorAction SilentlyContinue | Where-Object { Test-Path (Join-Path $_.FullName "bin\Host$hostArch\$targetArch\link.exe") }
    if (-not $found) {
        throw "the Visual Studio C++ $targetArch build tools are missing (bin\Host$hostArch\$targetArch\link.exe not found). In the Visual Studio Installer, add the component 'MSVC ... C++ ARM64/ARM64EC build tools (Latest)'."
    }
}

function Build-Platform($p) {
    $triple = $p.Triple
    $native = ($triple -eq $hostTriple) -or ($p.Os -eq $hostOs -and $p.Os -ne 'linux')
    if ($native) {
        if ($p.Os -eq 'windows' -and $triple -ne $hostTriple) { Test-MsvcCross $triple }
        Install-RustTarget $triple
        $cargoArgs = @('build', '--profile', $BuildProfile, '--locked')
        if ($triple -ne $hostTriple) { $cargoArgs += @('--target', $triple) }
        Write-Host "  cargo $($cargoArgs -join ' ')"
        Invoke-Native 'cargo build' { & cargo @cargoArgs }
    }
    else {
        if (-not (Test-Command 'zig')) { throw "zig is required to cross-compile $triple (winget install zig.zig)" }
        & cargo zigbuild --help *> $null
        if ($LASTEXITCODE -ne 0) { throw 'cargo-zigbuild is required (cargo install cargo-zigbuild)' }
        if ($p.Os -eq 'macos' -and -not $env:SDKROOT) {
            throw "a macOS SDK is required to cross-compile $triple : set SDKROOT to an extracted MacOSX.sdk folder, or build on a Mac"
        }
        Install-RustTarget $triple
        # glibc 2.17 : the binaries run on old Linux distributions too.
        $zigTriple = if ($p.Os -eq 'linux') { "$triple.2.17" } else { $triple }
        $cargoArgs = @('zigbuild', '--profile', $BuildProfile, '--locked', '--target', $zigTriple)
        Write-Host "  cargo $($cargoArgs -join ' ')"
        Invoke-Native 'cargo zigbuild' { & cargo @cargoArgs }
    }
    $dir = if ($triple -eq $hostTriple) { Join-Path 'target' $profileDir } else { Join-Path (Join-Path 'target' $triple) $profileDir }
    $bin = Join-Path $dir ($name + $p.Exe)
    if (-not (Test-Path $bin)) { throw "build succeeded but $bin was not found" }
    return $bin
}

# Zip with explicit entries so Unix permissions (rwxr-xr-x for the executable) are stored.
function New-ZipPackage {
    param([string]$ZipPath, [string]$RootName, [object[]]$Files, [bool]$Unix)
    Add-Type -AssemblyName System.IO.Compression
    if (Test-Path $ZipPath) { Remove-Item $ZipPath -Force }
    $fs = [System.IO.File]::Open($ZipPath, [System.IO.FileMode]::Create)
    $zip = [System.IO.Compression.ZipArchive]::new($fs, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($f in $Files) {
            $entry = $zip.CreateEntry("$RootName/$($f.Name)", [System.IO.Compression.CompressionLevel]::Optimal)
            $entry.LastWriteTime = [DateTimeOffset](Get-Item $f.Path).LastWriteTime
            if ($Unix) {
                $mode = [uint32]$(if ($f.Exec) { 0x81ED } else { 0x81A4 })   # -rwxr-xr-x / -rw-r--r--
                $entry.ExternalAttributes = [BitConverter]::ToInt32([BitConverter]::GetBytes([uint32]($mode -shl 16)), 0)
            }
            $out = $entry.Open()
            $in = [System.IO.File]::OpenRead($f.Path)
            try { $in.CopyTo($out) } finally { $in.Dispose(); $out.Dispose() }
        }
    }
    finally { $zip.Dispose(); $fs.Dispose() }
}

# ------------------------------------------------------------------ main
$publish = Join-Path $PSScriptRoot $OutDir
if ($Clean -and (Test-Path $publish)) { Remove-Item $publish -Recurse -Force }
New-Item -ItemType Directory -Force $publish | Out-Null

$results = @()
foreach ($key in $Platforms) {
    $p = $table[$key]
    $root = "$name-$version-$key"
    Write-Host "`n=== $key ($($p.Triple)) ===" -ForegroundColor Cyan
    $sw = [Diagnostics.Stopwatch]::StartNew()
    try {
        $bin = Build-Platform $p
        $files = @(@{ Path = (Resolve-Path $bin).Path; Name = $name + $p.Exe; Exec = $true })
        foreach ($extra in 'README.md', 'LICENSE', 'LICENSE.md', 'LICENSE.txt') {
            if (Test-Path $extra) { $files += @{ Path = (Resolve-Path $extra).Path; Name = $extra; Exec = $false } }
        }
        $zipPath = Join-Path $publish "$root.zip"
        New-ZipPackage -ZipPath $zipPath -RootName $root -Files $files -Unix ($p.Os -ne 'windows')
        $size = '{0:N1} MB' -f ((Get-Item $zipPath).Length / 1MB)
        Write-Host "  -> $zipPath ($size)" -ForegroundColor Green
        $results += [pscustomobject]@{ Platform = $key; Status = 'OK'; Package = "$root.zip"; Size = $size; Time = '{0:mm\:ss}' -f $sw.Elapsed }
    }
    catch {
        Write-Host "  FAILED: $($_.Exception.Message)" -ForegroundColor Red
        $results += [pscustomobject]@{ Platform = $key; Status = 'FAILED'; Package = $_.Exception.Message; Size = ''; Time = '{0:mm\:ss}' -f $sw.Elapsed }
    }
}

# Once every build is done (successful or not), empty the temporary build folders of the cross targets.
# target\<host>\release is the normal development build and is left alone (use `cargo clean` to remove it too).
if (-not $KeepBuild) {
    $freed = 0
    foreach ($key in $Platforms) {
        $triple = $table[$key].Triple
        $dir = Join-Path 'target' $triple
        if ($triple -ne $hostTriple -and (Test-Path $dir)) {
            $freed += (Get-ChildItem $dir -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
            Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    Write-Host ("`nTemporary build folders removed ({0:N1} GB freed)." -f ($freed / 1GB))
}

Write-Host "`n=== Summary ===" -ForegroundColor Cyan
$results | Format-Table -AutoSize -Wrap | Out-String | Write-Host
if ($results.Status -contains 'FAILED') { exit 1 }
