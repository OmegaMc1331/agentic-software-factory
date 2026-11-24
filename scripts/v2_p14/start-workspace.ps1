param(
    [Parameter(Mandatory = $true)][string]$ProjectDirectory,
    [ValidateRange(1, 65535)][int]$Port = 4321,
    [switch]$NoBrowser
)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath $ProjectDirectory).ProviderPath
if (-not (Test-Path -LiteralPath $projectRoot -PathType Container)) { throw 'Select an existing project parent directory' }
$logRoot = Join-Path $env:LOCALAPPDATA 'AgenticSoftwareFactory\logs'
New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
$logFile = Join-Path $logRoot ('workspace-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')
$arguments = @('start', '--workspace', '--no-default-project', '--allow-project-root', $projectRoot, '--port', $Port)
if ($NoBrowser) { $arguments += '--no-browser' }
Write-Host "Application log: $logFile"
& (Join-Path $PSScriptRoot 'factory.exe') @arguments 2>&1 | Tee-Object -FilePath $logFile
exit $LASTEXITCODE
