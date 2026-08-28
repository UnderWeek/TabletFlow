param(
    [string]$OutputDirectory = "dist",
    [string]$DaemonPath = $env:OTD_DAEMON_PATH,
    [string]$TabletFlowBinaryPath = $(if ($env:TABLETFLOW_BINARY_PATH) { $env:TABLETFLOW_BINARY_PATH } else { Join-Path $PWD "target/release/tabletflow.exe" }),
    [string]$Version = $(if ($env:TABLETFLOW_VERSION) { $env:TABLETFLOW_VERSION } else { "0.1.0" })
)

$ErrorActionPreference = "Stop"
$RootDirectory = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$OutputDirectory = Join-Path $RootDirectory $OutputDirectory
$StagingDirectory = Join-Path $OutputDirectory "TabletFlow-$Version-windows"
$ArchivePath = Join-Path $OutputDirectory "TabletFlow-$Version-windows.zip"

if ([string]::IsNullOrWhiteSpace($DaemonPath) -or -not (Test-Path $DaemonPath -PathType Leaf)) {
    throw "OTD_DAEMON_PATH must point to OpenTabletDriver.Daemon.exe"
}
if (-not (Test-Path $TabletFlowBinaryPath -PathType Leaf)) {
    throw "TABLETFLOW_BINARY_PATH must point to TabletFlow.exe"
}

if (Test-Path $StagingDirectory) { Remove-Item $StagingDirectory -Recurse -Force }
if (Test-Path $ArchivePath) { Remove-Item $ArchivePath -Force }
New-Item $StagingDirectory -ItemType Directory -Force | Out-Null

Copy-Item $TabletFlowBinaryPath (Join-Path $StagingDirectory "TabletFlow.exe")
Copy-Item $DaemonPath (Join-Path $StagingDirectory "OpenTabletDriver.Daemon.exe")
if ($env:OTD_LICENSE_PATH -and (Test-Path $env:OTD_LICENSE_PATH -PathType Leaf)) {
    Copy-Item $env:OTD_LICENSE_PATH (Join-Path $StagingDirectory "OpenTabletDriver.LICENSE.txt")
}
Compress-Archive -Path (Join-Path $StagingDirectory "*") -DestinationPath $ArchivePath
Remove-Item $StagingDirectory -Recurse -Force

Write-Output $ArchivePath
