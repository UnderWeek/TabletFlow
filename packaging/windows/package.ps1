param(
    [string]$OutputDirectory = "dist",
    [string]$DaemonDirectory = $env:OTD_DAEMON_DIR,
    [string]$TabletFlowBinaryPath = $(if ($env:TABLETFLOW_BINARY_PATH) { $env:TABLETFLOW_BINARY_PATH } else { Join-Path $PWD "target/release/tabletflow.exe" }),
    [string]$Version = $(if ($env:TABLETFLOW_VERSION) { $env:TABLETFLOW_VERSION } else { "0.1.0" })
)

$ErrorActionPreference = "Stop"
$RootDirectory = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$OutputDirectory = Join-Path $RootDirectory $OutputDirectory
$StagingDirectory = Join-Path $OutputDirectory "TabletFlow-$Version-windows"
$ArchivePath = Join-Path $OutputDirectory "TabletFlow-$Version-windows.zip"

# The daemon is published as a plain multi-file output (not PublishSingleFile) on
# Windows to avoid the self-extraction cold-start delay, so the whole output
# directory ships alongside TabletFlow.exe instead of a single daemon .exe.
if ([string]::IsNullOrWhiteSpace($DaemonDirectory) -or -not (Test-Path $DaemonDirectory -PathType Container) -or -not (Test-Path (Join-Path $DaemonDirectory "OpenTabletDriver.Daemon.exe") -PathType Leaf)) {
    throw "OTD_DAEMON_DIR must point to the directory containing OpenTabletDriver.Daemon.exe and its dependencies"
}
if (-not (Test-Path $TabletFlowBinaryPath -PathType Leaf)) {
    throw "TABLETFLOW_BINARY_PATH must point to TabletFlow.exe"
}

if (Test-Path $StagingDirectory) { Remove-Item $StagingDirectory -Recurse -Force }
if (Test-Path $ArchivePath) { Remove-Item $ArchivePath -Force }
New-Item $StagingDirectory -ItemType Directory -Force | Out-Null

Copy-Item $TabletFlowBinaryPath (Join-Path $StagingDirectory "TabletFlow.exe")
Copy-Item (Join-Path $DaemonDirectory "*") $StagingDirectory -Recurse
if ($env:OTD_LICENSE_PATH -and (Test-Path $env:OTD_LICENSE_PATH -PathType Leaf)) {
    Copy-Item $env:OTD_LICENSE_PATH (Join-Path $StagingDirectory "OpenTabletDriver.LICENSE.txt")
}
Compress-Archive -Path (Join-Path $StagingDirectory "*") -DestinationPath $ArchivePath
Remove-Item $StagingDirectory -Recurse -Force

Write-Output $ArchivePath
