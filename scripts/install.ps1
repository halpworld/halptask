# HalpTask PowerShell Installer Script for Windows
# ⚠️ WARNING: Never trust install scripts blindly! Always inspect the source code before executing.
# Usage: irm https://raw.githubusercontent.com/halpworld/halptask/main/scripts/install.ps1 | iex

$ErrorActionPreference = 'Stop'

$Repo = "halpworld/halptask"
$BinaryName = "halptask.exe"

Write-Host "🚀 Installing HalpTask for Windows..." -ForegroundColor Cyan

# 1. Detect Architecture
$ArchRaw = $env:PROCESSOR_ARCHITECTURE
switch -Wildcard ($ArchRaw) {
    "*ARM64*" { $Arch = "arm64" }
    default   { $Arch = "x86_64" }
}

# 2. Determine Version & Download URL
Write-Host "🔍 Finding latest release..." -ForegroundColor Yellow
try {
    $ReleaseInfo = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
    $Tag = $ReleaseInfo.tag_name
} catch {
    $Tag = $null
}

if ($Tag) {
    $DownloadUrlPrimary = "https://github.com/$Repo/releases/download/$Tag/halptask_Windows_$Arch.exe"
    $DownloadUrlFallback = "https://github.com/$Repo/releases/download/$Tag/halptask_Windows_$Arch"
} else {
    $DownloadUrlPrimary = "https://github.com/$Repo/releases/latest/download/halptask_Windows_$Arch.exe"
    $DownloadUrlFallback = "https://github.com/$Repo/releases/latest/download/halptask_Windows_$Arch"
}

# 3. Determine Destination Directory
$InstallDir = Join-Path $env:USERPROFILE "bin"
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

$TargetPath = Join-Path $InstallDir $BinaryName

# 4. Download Binary
$TempFile = [System.IO.Path]::GetTempFileName()
Write-Host "📥 Downloading binary for Windows/$Arch..." -ForegroundColor Cyan

try {
    Invoke-WebRequest -Uri $DownloadUrlPrimary -OutFile $TempFile -UseBasicParsing
} catch {
    try {
        Invoke-WebRequest -Uri $DownloadUrlFallback -OutFile $TempFile -UseBasicParsing
    } catch {
        Write-Host "❌ Download failed! Could not download from release assets." -ForegroundColor Red
        Remove-Item -Path $TempFile -ErrorAction SilentlyContinue
        exit 1
    }
}

# 5. Install Binary
Write-Host "📦 Installing to $TargetPath..." -ForegroundColor Cyan
Move-Item -Path $TempFile -Destination $TargetPath -Force

Write-Host "✅ HalpTask successfully installed!" -ForegroundColor Green
Write-Host "🎉 Run 'halptask' (ensure '$InstallDir' is in your PATH) to get started." -ForegroundColor Green
