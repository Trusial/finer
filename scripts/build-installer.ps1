#Requires -Version 5.1
param(
    [string]$MediaTools = ""
)

$ErrorActionPreference = "Stop"
$repositoryRoot = Split-Path $PSScriptRoot
$targetTriple = "x86_64-pc-windows-msvc"
$sidecarDirectory = Join-Path $repositoryRoot "src-tauri\bin"

if (-not $MediaTools) {
    $MediaTools = Join-Path $repositoryRoot "media-tools\bin"
}

$ffmpeg = Join-Path $MediaTools "ffmpeg.exe"
$ffprobe = Join-Path $MediaTools "ffprobe.exe"
foreach ($tool in @($ffmpeg, $ffprobe)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) {
        throw "Missing $tool. Build the media tools first or pass -MediaTools with a folder containing ffmpeg.exe and ffprobe.exe."
    }
}

New-Item -ItemType Directory -Path $sidecarDirectory -Force | Out-Null
Copy-Item -LiteralPath $ffmpeg -Destination (Join-Path $sidecarDirectory "ffmpeg-$targetTriple.exe") -Force
Copy-Item -LiteralPath $ffprobe -Destination (Join-Path $sidecarDirectory "ffprobe-$targetTriple.exe") -Force

Push-Location -LiteralPath $repositoryRoot
try {
    npm run tauri -- build --bundles nsis --ci
    if ($LASTEXITCODE -ne 0) {
        throw "Tauri installer build failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}
