[CmdletBinding()]
param([string]$Cargo = 'cargo', [string]$CoreArchive, [string]$DestinationRoot)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$version = '26.11.9'
$sha256 = '1262522266337b824f07beb8e1375786b437683b3890105469088f8e17e0a69d'
if (!$DestinationRoot) { $DestinationRoot = Join-Path $root 'dist' }
$DestinationRoot = [System.IO.Path]::GetFullPath($DestinationRoot)
$appVersion = 'v0.2.0-beta.1'
$target = Join-Path $DestinationRoot 'sh1n VPN'
$core = Join-Path $target 'core'
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("velum-build-" + [guid]::NewGuid())

Push-Location $root
try {
    & $Cargo test --locked
    if ($LASTEXITCODE) { throw "cargo test failed: $LASTEXITCODE" }
    & $Cargo build --release --locked
    if ($LASTEXITCODE) { throw "cargo build failed: $LASTEXITCODE" }

    New-Item -ItemType Directory -Path $core, $scratch -Force | Out-Null
    $bundle = Join-Path $scratch 'xray.zip'
    if ($CoreArchive) {
        Copy-Item -LiteralPath $CoreArchive -Destination $bundle
    } else {
        Invoke-WebRequest -Uri "https://github.com/XTLS/Xray-core/releases/download/v$version/Xray-windows-64.zip" -OutFile $bundle -UseBasicParsing
    }
    if ((Get-FileHash -LiteralPath $bundle -Algorithm SHA256).Hash -ne $sha256) {
        throw 'Official Xray archive SHA-256 mismatch.'
    }
    Expand-Archive -LiteralPath $bundle -DestinationPath $scratch -Force
    $files = 'xray.exe','wintun.dll','geoip.dat','geosite.dat','LICENSE','LICENSE-Wintun','README.md'
    foreach ($name in $files) {
        Copy-Item -LiteralPath (Join-Path $scratch $name) -Destination $core -Force
    }
    Copy-Item -LiteralPath 'target\release\sh1n-vpn.exe' -Destination (Join-Path $target 'sh1n VPN.exe') -Force
    Copy-Item -LiteralPath 'docs\usage.md' -Destination (Join-Path $target 'START-HERE.md') -Force
    Copy-Item -LiteralPath 'THEMES.md' -Destination $target -Force
    $notices = Join-Path $target 'THIRD-PARTY-RUST.txt'
    $metadata = (& $Cargo metadata --locked --format-version 1 | ConvertFrom-Json)
    if ($LASTEXITCODE) { throw 'cargo metadata failed' }
    $text = [System.Collections.Generic.List[string]]::new()
    $text.Add('Third-party Rust dependency notices. Rust standard library: https://github.com/rust-lang/rust.')
    foreach ($package in ($metadata.packages | Where-Object source | Sort-Object name,version)) {
        $text.Add("")
        $text.Add("=== $($package.name) $($package.version) | $($package.license) | $($package.repository) ===")
        $dir = Split-Path -Parent $package.manifest_path
        foreach ($license in (Get-ChildItem -LiteralPath $dir -File | Where-Object Name -Match '^(LICENSE|COPYING|NOTICE|UNLICENSE)([-_.].*)?$')) {
            $text.Add("[$($license.Name)]")
            $text.Add([System.IO.File]::ReadAllText($license.FullName))
        }
    }
    [System.IO.File]::WriteAllLines($notices,$text)
    $checkLog = Join-Path $scratch 'core-check.txt'
    $process = Start-Process -FilePath (Join-Path $target 'sh1n VPN.exe') -ArgumentList '--check-core' -WindowStyle Hidden -Wait -PassThru -RedirectStandardError $checkLog
    if ($process.ExitCode) { Get-Content -LiteralPath $checkLog; throw "Xray config check failed: $($process.ExitCode)" }

    $items = @((Join-Path $target 'sh1n VPN.exe'), (Join-Path $target 'START-HERE.md'), (Join-Path $target 'THEMES.md'), $notices) +
        @($files | ForEach-Object { Join-Path $core $_ })
    $release = Join-Path $DestinationRoot "sh1n-VPN-$appVersion-Windows-x64.zip"
    # Explicit manifest: never package the user's DPAPI store.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $output = [System.IO.File]::Open($release,[System.IO.FileMode]::Create)
    try {
        $zip = [System.IO.Compression.ZipArchive]::new($output,[System.IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($file in $items) {
                $entry = 'sh1n VPN/' + $file.Substring($target.Length + 1).Replace('\','/')
                [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip,$file,$entry) | Out-Null
            }
        } finally { $zip.Dispose() }
    } finally { $output.Dispose() }
    $sha = (Get-FileHash -LiteralPath $release -Algorithm SHA256).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText("$release.sha256", "$sha  $([System.IO.Path]::GetFileName($release))" + [Environment]::NewLine)
    Write-Output $release
} finally {
    Pop-Location
    if (Test-Path -LiteralPath $scratch -PathType Container) {
        Get-ChildItem -LiteralPath $scratch -File | Remove-Item
        if (-not (Get-ChildItem -LiteralPath $scratch -Force)) { Remove-Item -LiteralPath $scratch }
    }
}
