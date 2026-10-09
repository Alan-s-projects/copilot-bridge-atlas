[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$atlasRoot = Split-Path -Parent $PSScriptRoot
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::Is64BitOperatingSystem) {
    throw 'Atlas builds only a Windows x64 MSI.'
}

Push-Location -LiteralPath $atlasRoot
try {
    $atlasVersion = (Get-Content -LiteralPath package.json -Raw | ConvertFrom-Json).version
    $tauriVersion = (Get-Content -LiteralPath src-tauri\tauri.conf.json -Raw | ConvertFrom-Json).version
    $cargoVersion = [regex]::Match(
        (Get-Content -LiteralPath src-tauri\Cargo.toml -Raw),
        '(?m)^version = "([^"]+)"'
    ).Groups[1].Value
    if ($atlasVersion -notmatch '^\d+\.\d+\.\d+$' -or
        $atlasVersion -ne $tauriVersion -or $atlasVersion -ne $cargoVersion) {
        throw 'package.json, Cargo.toml and tauri.conf.json must share one stable version.'
    }
    $rustInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0 -or $rustInfo -notcontains 'host: x86_64-pc-windows-msvc') {
        throw 'Use the x86_64-pc-windows-msvc Rust toolchain.'
    }
    if ($env:CARGO_BUILD_TARGET -and $env:CARGO_BUILD_TARGET -ne 'x86_64-pc-windows-msvc') {
        throw 'CARGO_BUILD_TARGET must be x86_64-pc-windows-msvc or unset.'
    }

    & node node_modules/@tauri-apps/cli/tauri.js build --bundles msi
    if ($LASTEXITCODE -ne 0) { throw "MSI build failed ($LASTEXITCODE)." }

    $targetRoot = if ($env:CARGO_TARGET_DIR) {
        [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    } else {
        Join-Path $atlasRoot 'src-tauri\target'
    }
    if ($env:CARGO_BUILD_TARGET) {
        $targetRoot = Join-Path $targetRoot $env:CARGO_BUILD_TARGET
    }
    $builtMsi = Join-Path $targetRoot "release\bundle\msi\Copilot Bridge Atlas_${atlasVersion}_x64_en-US.msi"
    if (-not (Test-Path -LiteralPath $builtMsi -PathType Leaf)) {
        throw "Expected installer is missing: $builtMsi"
    }
    & (Join-Path $PSScriptRoot 'check-msi-shortcuts.ps1') -Path $builtMsi
    $releaseDir = Join-Path $atlasRoot 'release'
    New-Item -ItemType Directory -Path $releaseDir -Force | Out-Null
    $installerName = "Copilot-Bridge-Atlas-$atlasVersion-Windows-x64.msi"
    $installerPath = Join-Path $releaseDir $installerName
    Copy-Item -LiteralPath $builtMsi -Destination $installerPath -Force
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $installerStream = [IO.File]::OpenRead($installerPath)
        try {
            $hash = [BitConverter]::ToString(
                $sha256.ComputeHash($installerStream)
            ).Replace('-', '').ToLowerInvariant()
        } finally {
            $installerStream.Dispose()
        }
    } finally {
        $sha256.Dispose()
    }
    [IO.File]::WriteAllText("$installerPath.sha256", "$hash  $installerName`n")
    Write-Output $installerPath
    Write-Output "SHA256: $hash"
} finally {
    Pop-Location
}
