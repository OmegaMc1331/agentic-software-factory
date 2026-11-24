param([Parameter(Mandatory = $true)][string]$Archive, [Parameter(Mandatory = $true)][string]$Destination)
$ErrorActionPreference = 'Stop'
if ((Test-Path -LiteralPath $Destination) -and @(Get-ChildItem -LiteralPath $Destination -Force).Count -gt 0) {
  if ((Get-Content -LiteralPath (Join-Path $Destination '.asf-p01-package') -Raw) -ne 'ASF_P01_PACKAGE_V1') { throw 'Refusing an unowned destination' }
}
Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force
