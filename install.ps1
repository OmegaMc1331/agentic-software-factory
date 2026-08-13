#requires -Version 5.1
#
# Agentic Software Factory installer (Windows).
#
# Installs the latest prebuilt factory.exe for the current user. No
# administrator rights, Rust, Node or Git required.
#
# Main usage, from any PowerShell:
#
#   irm https://raw.githubusercontent.com/OmegaMc1331/agentic-software-factory/main/install.ps1 | iex
#
# Optional environment overrides (mainly for pinning a version or testing):
#   FACTORY_VERSION      install a specific release tag, e.g. v0.1.0
#   FACTORY_BASE_URL     download the archive and checksum from this base URL
#   FACTORY_INSTALL_DIR  install into this directory (default is under
#                        %LOCALAPPDATA%; a custom directory skips PATH setup)
#   FACTORY_DRY_RUN=1    resolve everything and print what would happen, install nothing

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Get-AsfInstallDir {
  if ($env:FACTORY_INSTALL_DIR) {
    return [IO.Path]::GetFullPath($env:FACTORY_INSTALL_DIR)
  }
  return (Join-Path $env:LOCALAPPDATA 'Programs\AgenticSoftwareFactory\bin')
}

function Get-AsfReleaseTag {
  if ($env:FACTORY_VERSION) {
    return $env:FACTORY_VERSION
  }
  $apiUrl = 'https://api.github.com/repos/OmegaMc1331/agentic-software-factory/releases/latest'
  try {
    $release = Invoke-RestMethod -Uri $apiUrl -Headers @{ 'User-Agent' = 'factory-installer' }
  } catch {
    throw 'Could not find a release. Check the repository and your network connection.'
  }
  return $release.tag_name
}

function Test-AsfOnUserPath {
  param([string]$Directory)
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $matches = @($userPath -split ';' | Where-Object {
    $_ -and $_.TrimEnd('\') -ieq $Directory.TrimEnd('\')
  })
  return $matches.Count -gt 0
}

$installDir = Get-AsfInstallDir
$exePath = Join-Path $installDir 'factory.exe'
$asset = 'factory-windows-x86_64.zip'

# 1. detect the architecture
# PROCESSOR_ARCHITECTURE is Windows-only; when it is unset (for example when
# the script is run under pwsh on another platform), use the one published
# architecture.
$procArch = if ($env:PROCESSOR_ARCHITECTURE) { $env:PROCESSOR_ARCHITECTURE } else { 'AMD64' }
switch -Regex ($procArch) {
  '^(AMD64|x86_64)$' { $arch = 'x86_64' }
  '^ARM64$' {
    $arch = 'x86_64'
    Write-Host 'Note: Windows on ARM — installing the x86_64 build (runs via emulation).'
  }
  default {
    throw "Unsupported architecture: $procArch. Only x86_64 releases are published."
  }
}

# 2. find the release to install (a pinned version needs no network lookup)
if (-not $env:FACTORY_VERSION) {
  Write-Host 'Looking up the latest Agentic Software Factory release...'
}
$tag = Get-AsfReleaseTag

$base = if ($env:FACTORY_BASE_URL) {
  $env:FACTORY_BASE_URL.TrimEnd('/')
} else {
  "https://github.com/OmegaMc1331/agentic-software-factory/releases/download/$tag"
}

if ($env:FACTORY_DRY_RUN -eq '1') {
  Write-Host "dry-run: would install $tag"
  Write-Host "  download  $base/$asset"
  Write-Host "  install   factory.exe -> $exePath"
  return
}

# 3. download the archive and its published checksum
$tmp = Join-Path ([IO.Path]::GetTempPath()) "factory-install-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $tmp | Out-Null
$zipPath = Join-Path $tmp $asset
$shaPath = Join-Path $tmp "$asset.sha256"

Write-Host "Downloading $asset ($tag)..."
Invoke-WebRequest -Uri "$base/$asset" -OutFile $zipPath
Invoke-WebRequest -Uri "$base/$asset.sha256" -OutFile $shaPath
if (-not (Test-Path $shaPath)) {
  Remove-Item -Recurse -Force $tmp
  throw 'No published checksum for this release. Refusing to install an unverified binary.'
}

# 4. verify the SHA-256 checksum
$expected = ((Get-Content $shaPath -Raw).Trim() -split '\s+')[0]
$actual = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash
if (-not $expected -or $actual -ine $expected) {
  Remove-Item -Recurse -Force $tmp
  throw "Checksum mismatch for $asset. Expected $expected, got $actual. Aborting."
}
Write-Host 'Checksum OK.'

# 5. extract factory.exe and install it
Expand-Archive -Path $zipPath -DestinationPath $tmp -Force
$extracted = Join-Path $tmp 'factory.exe'
if (-not (Test-Path $extracted)) {
  Remove-Item -Recurse -Force $tmp
  throw 'The archive does not contain factory.exe.'
}
New-Item -ItemType Directory -Path $installDir -Force | Out-Null
Copy-Item -Path $extracted -Destination $exePath -Force

# 6. make sure the install directory is on the user PATH
if (-not $env:FACTORY_INSTALL_DIR) {
  if (-not (Test-AsfOnUserPath $installDir)) {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $installDir } else { "$installDir;$userPath" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    $env:Path = "$installDir;$env:Path"
    $pathChanged = $true
  }
}

# 7. clean up
Remove-Item -Recurse -Force $tmp

# 8. done
Write-Host ''
Write-Host "Installed factory $tag to $exePath"
if ($pathChanged) {
  Write-Host 'Added to your user PATH. Open a new terminal, then run: factory --version'
} elseif ($env:FACTORY_INSTALL_DIR) {
  Write-Host "Add $installDir to your PATH, then run: factory --version"
} else {
  Write-Host 'The install directory is already on your user PATH. Run: factory --version'
}