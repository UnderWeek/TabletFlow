param(
    [string]$OutputDirectory = "dist",
    [string]$DaemonDirectory = $env:OTD_DAEMON_DIR,
    [string]$TabletFlowBinaryPath = $(if ($env:TABLETFLOW_BINARY_PATH) { $env:TABLETFLOW_BINARY_PATH } else { Join-Path $PWD "target/release/tabletflow.exe" }),
    [string]$LicensePath = $env:OTD_LICENSE_PATH,
    [string]$Version = $(if ($env:TABLETFLOW_VERSION) { $env:TABLETFLOW_VERSION } else { "0.1.0" }),
    [int]$MinimumDaemonFileCount = 200
)

$ErrorActionPreference = "Stop"
$RootDirectory = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-FileFingerprint {
    param([Parameter(Mandatory = $true)][string]$Path)

    $file = Get-Item -LiteralPath $Path
    $hash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    return "$($file.Length):$hash"
}

function Get-DirectoryManifest {
    param([Parameter(Mandatory = $true)][string]$Directory)

    $resolvedDirectory = (Resolve-Path -LiteralPath $Directory).Path
    $manifest = @{}
    foreach ($file in Get-ChildItem -LiteralPath $resolvedDirectory -File -Recurse -Force | Sort-Object FullName) {
        $relativePath = [IO.Path]::GetRelativePath($resolvedDirectory, $file.FullName).Replace('\', '/')
        if ($manifest.ContainsKey($relativePath)) {
            throw "Duplicate file path while creating manifest: $relativePath"
        }
        $manifest[$relativePath] = Get-FileFingerprint -Path $file.FullName
    }
    return ,$manifest
}

function Get-ZipManifest {
    param([Parameter(Mandatory = $true)][string]$Path)

    $manifest = @{}
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        foreach ($entry in $archive.Entries) {
            if ([string]::IsNullOrEmpty($entry.Name)) {
                continue
            }

            $relativePath = $entry.FullName.Replace('\', '/')
            if ($manifest.ContainsKey($relativePath)) {
                throw "Duplicate file path in ZIP archive: $relativePath"
            }

            $stream = $entry.Open()
            $sha256 = [Security.Cryptography.SHA256]::Create()
            try {
                $hashBytes = $sha256.ComputeHash($stream)
                $hash = ([BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
            }
            finally {
                $sha256.Dispose()
                $stream.Dispose()
            }
            $manifest[$relativePath] = "$($entry.Length):$hash"
        }
    }
    finally {
        $archive.Dispose()
    }
    return ,$manifest
}

function Assert-ManifestsEqual {
    param(
        [Parameter(Mandatory = $true)][hashtable]$Expected,
        [Parameter(Mandatory = $true)][hashtable]$Actual,
        [Parameter(Mandatory = $true)][string]$Label
    )

    $missing = @($Expected.Keys | Where-Object { -not $Actual.ContainsKey($_) } | Sort-Object)
    $unexpected = @($Actual.Keys | Where-Object { -not $Expected.ContainsKey($_) } | Sort-Object)
    $changed = @($Expected.Keys | Where-Object { $Actual.ContainsKey($_) -and $Expected[$_] -ne $Actual[$_] } | Sort-Object)
    if ($missing.Count -gt 0 -or $unexpected.Count -gt 0 -or $changed.Count -gt 0) {
        $details = @()
        if ($missing.Count -gt 0) { $details += "missing: $($missing -join ', ')" }
        if ($unexpected.Count -gt 0) { $details += "unexpected: $($unexpected -join ', ')" }
        if ($changed.Count -gt 0) { $details += "content mismatch: $($changed -join ', ')" }
        throw "$Label manifest mismatch ($($details -join '; '))"
    }
}

function Assert-DaemonDependencyClosure {
    param(
        [Parameter(Mandatory = $true)][string]$Directory,
        [Parameter(Mandatory = $true)][hashtable]$Manifest
    )

    $depsPath = Join-Path $Directory "OpenTabletDriver.Daemon.deps.json"
    $deps = Get-Content -LiteralPath $depsPath -Raw | ConvertFrom-Json -AsHashtable
    $runtimeTargetName = $deps["runtimeTarget"]["name"]
    if ([string]::IsNullOrWhiteSpace($runtimeTargetName) -or -not $deps["targets"].ContainsKey($runtimeTargetName)) {
        throw "OpenTabletDriver.Daemon.deps.json does not contain its declared runtime target"
    }
    if (-not $runtimeTargetName.EndsWith("/win-x64")) {
        throw "OpenTabletDriver daemon was published for '$runtimeTargetName', expected win-x64"
    }

    $missingAssets = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($library in $deps["targets"][$runtimeTargetName].Values) {
        foreach ($sectionName in @("runtime", "native", "resources")) {
            if (-not $library.ContainsKey($sectionName)) {
                continue
            }
            foreach ($asset in $library[$sectionName].GetEnumerator()) {
                $fileName = ($asset.Key -split '/')[-1]
                $relativePath = if ($sectionName -eq "resources") {
                    "$($asset.Value['locale'])/$fileName"
                }
                else {
                    $fileName
                }
                if (-not $Manifest.ContainsKey($relativePath)) {
                    [void]$missingAssets.Add($relativePath)
                }
            }
        }
    }

    if ($missingAssets.Count -gt 0) {
        throw "OpenTabletDriver publish output is missing assets declared by deps.json: $(@($missingAssets) -join ', ')"
    }
}

if ([string]::IsNullOrWhiteSpace($DaemonDirectory) -or -not (Test-Path -LiteralPath $DaemonDirectory -PathType Container)) {
    throw "OTD_DAEMON_DIR must point to the multi-file OpenTabletDriver publish directory"
}
if (-not (Test-Path -LiteralPath $TabletFlowBinaryPath -PathType Leaf)) {
    throw "TABLETFLOW_BINARY_PATH must point to TabletFlow.exe"
}
if ([string]::IsNullOrWhiteSpace($LicensePath) -or -not (Test-Path -LiteralPath $LicensePath -PathType Leaf)) {
    throw "OTD_LICENSE_PATH must point to the OpenTabletDriver license"
}
if ($MinimumDaemonFileCount -lt 1) {
    throw "MinimumDaemonFileCount must be positive"
}

$DaemonDirectory = (Resolve-Path -LiteralPath $DaemonDirectory).Path
$TabletFlowBinaryPath = (Resolve-Path -LiteralPath $TabletFlowBinaryPath).Path
$LicensePath = (Resolve-Path -LiteralPath $LicensePath).Path
if (-not [IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory = Join-Path $RootDirectory $OutputDirectory
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$StagingDirectory = Join-Path $OutputDirectory "TabletFlow-$Version-windows"
$ArchivePath = Join-Path $OutputDirectory "TabletFlow-$Version-windows.zip"

$requiredDaemonFiles = @(
    "OpenTabletDriver.Daemon.exe",
    "OpenTabletDriver.Daemon.dll",
    "OpenTabletDriver.Daemon.deps.json",
    "OpenTabletDriver.Daemon.runtimeconfig.json",
    "OpenTabletDriver.dll",
    "OpenTabletDriver.Configurations.dll",
    "OpenTabletDriver.Desktop.dll",
    "OpenTabletDriver.Native.dll",
    "OpenTabletDriver.Plugin.dll",
    "HidSharpCore.dll",
    "coreclr.dll",
    "hostfxr.dll",
    "hostpolicy.dll"
)
foreach ($relativePath in $requiredDaemonFiles) {
    if (-not (Test-Path -LiteralPath (Join-Path $DaemonDirectory $relativePath) -PathType Leaf)) {
        throw "OpenTabletDriver multi-file publish is incomplete: missing $relativePath"
    }
}

$daemonManifest = Get-DirectoryManifest -Directory $DaemonDirectory
if ($daemonManifest.Count -lt $MinimumDaemonFileCount) {
    throw "OpenTabletDriver publish contains only $($daemonManifest.Count) files; expected at least $MinimumDaemonFileCount for the win-x64 multi-file runtime"
}
Assert-DaemonDependencyClosure -Directory $DaemonDirectory -Manifest $daemonManifest

$expectedManifest = @{}
foreach ($entry in $daemonManifest.GetEnumerator()) {
    $expectedManifest[$entry.Key] = $entry.Value
}
$additionalFiles = @{
    "TabletFlow.exe" = $TabletFlowBinaryPath
    "OpenTabletDriver.LICENSE.txt" = $LicensePath
}
foreach ($entry in $additionalFiles.GetEnumerator()) {
    if ($expectedManifest.ContainsKey($entry.Key)) {
        throw "OpenTabletDriver publish unexpectedly contains reserved package path: $($entry.Key)"
    }
    $expectedManifest[$entry.Key] = Get-FileFingerprint -Path $entry.Value
}

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
if (Test-Path -LiteralPath $StagingDirectory) { Remove-Item -LiteralPath $StagingDirectory -Recurse -Force }
if (Test-Path -LiteralPath $ArchivePath) { Remove-Item -LiteralPath $ArchivePath -Force }
New-Item -Path $StagingDirectory -ItemType Directory -Force | Out-Null

foreach ($item in Get-ChildItem -LiteralPath $DaemonDirectory -Force) {
    Copy-Item -LiteralPath $item.FullName -Destination $StagingDirectory -Recurse -Force
}
Copy-Item -LiteralPath $TabletFlowBinaryPath -Destination (Join-Path $StagingDirectory "TabletFlow.exe")
Copy-Item -LiteralPath $LicensePath -Destination (Join-Path $StagingDirectory "OpenTabletDriver.LICENSE.txt")

$stagingManifest = Get-DirectoryManifest -Directory $StagingDirectory
Assert-ManifestsEqual -Expected $expectedManifest -Actual $stagingManifest -Label "Windows staging directory"

[IO.Compression.ZipFile]::CreateFromDirectory(
    $StagingDirectory,
    $ArchivePath,
    [IO.Compression.CompressionLevel]::Optimal,
    $false
)
$zipManifest = Get-ZipManifest -Path $ArchivePath
Assert-ManifestsEqual -Expected $stagingManifest -Actual $zipManifest -Label "Windows ZIP"

Remove-Item -LiteralPath $StagingDirectory -Recurse -Force
Write-Host "Validated Windows package: $($zipManifest.Count) files, exact source/staging/ZIP match."
Write-Output $ArchivePath
