param([Parameter(Mandatory = $true)][string]$ProjectRoot, [int]$Port = 0)
$ErrorActionPreference = 'Stop'
$packageRoot = $PSScriptRoot
$fixtureRoot = (Resolve-Path -LiteralPath $ProjectRoot).Path
if ((Get-Content -LiteralPath (Join-Path $fixtureRoot '.asf-p01-fixture') -Raw) -ne 'ASF_P01_OWNED_FIXTURE_V1') { throw 'Only owned P01 fixtures are accepted' }
$stateRoot = Join-Path $fixtureRoot '.factory/p01'
New-Item -ItemType Directory -Force -Path $stateRoot | Out-Null
$configPath = Join-Path $stateRoot 'package-config.json'
$configuration = @{
  projects = @{ small = $fixtureRoot }
  node = Join-Path $packageRoot 'runtime/node.exe'
  python = Join-Path $packageRoot 'runtime/python/python.exe'
  lsp = Join-Path $packageRoot 'tools/node_modules/typescript-language-server/lib/cli.mjs'
  pyright = Join-Path $packageRoot 'tools/node_modules/pyright/langserver.index.js'
  probe = Join-Path $packageRoot 'terminal_probe.py'
  assets = Join-Path $packageRoot 'no-external-assets'
}
[System.IO.File]::WriteAllText($configPath, ($configuration | ConvertTo-Json -Depth 5), [System.Text.UTF8Encoding]::new($false))
& (Join-Path $packageRoot 'asf-p01-ide-probe.exe') $configPath $Port
exit $LASTEXITCODE
