#Requires -Version 5.1
param(
    [string]$MediaTools = "",
    [string]$OutputDirectory = ""
)

$ErrorActionPreference = "Stop"
$repositoryRoot = Split-Path $PSScriptRoot
$buildTarget = "x86_64-pc-windows-msvc"

if (-not $MediaTools) {
    $MediaTools = Join-Path $repositoryRoot "media-tools"
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path $repositoryRoot "release"
}

$MediaTools = (Resolve-Path -LiteralPath $MediaTools).Path

function Invoke-Checked {
    param(
        [scriptblock]$Command
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "Build command exited with code $LASTEXITCODE."
    }
}

function Assert-X64Executable {
    param(
        [string]$Path
    )

    $stream = [System.IO.File]::OpenRead($Path)
    $reader = [System.IO.BinaryReader]::new($stream)
    try {
        if ($reader.ReadUInt16() -ne 0x5a4d) {
            throw "Invalid executable $Path"
        }

        $stream.Position = 0x3c
        $stream.Position = $reader.ReadUInt32()
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "Expected a Windows x64 executable at $Path"
        }
    } finally {
        $reader.Dispose()
    }
}

foreach ($required in @(
    "bin\ffmpeg.exe",
    "bin\ffprobe.exe",
    "licenses\COPYING.GPLv2",
    "licenses\x264-GPL-2.0.txt",
    "licenses\media-build.txt",
    "source\build-media-tools.sh"
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $MediaTools $required) -PathType Leaf)) {
        throw "Missing $required. Run scripts/build-media-tools.sh first."
    }
}

foreach ($archive in @("ffmpeg-*.tar.gz", "x264-*.tar.gz", "nv-codec-headers-*.tar.gz")) {
    $files = @(Get-ChildItem (Join-Path $MediaTools "source") -File -Filter $archive)
    if ($files.Count -ne 1) {
        throw "Expected one source archive matching $archive. Rebuild the tools with scripts/build-media-tools.sh."
    }
}

$ffmpeg = Join-Path $MediaTools "bin\ffmpeg.exe"
$ffprobe = Join-Path $MediaTools "bin\ffprobe.exe"
Assert-X64Executable $ffmpeg
Assert-X64Executable $ffprobe

Push-Location -LiteralPath $repositoryRoot
try {
    Invoke-Checked { npm ci }
    Invoke-Checked { cargo test --locked -p finer-engine }
    Invoke-Checked { npm run tauri -- build --target $buildTarget --no-bundle --ci -- --locked }

    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) {
        throw "Could not resolve the Cargo target directory."
    }

    $executable = Join-Path $metadata.target_directory "$buildTarget\release\finer.exe"
    Assert-X64Executable $executable

    $version = ($metadata.packages | Where-Object { $_.name -eq "finer" }).version
    $outputDirectoryPath = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
    $temporaryBuild = Join-Path ([System.IO.Path]::GetTempPath()) ("finer-" + [guid]::NewGuid().ToString("N"))
    $applicationDirectory = Join-Path $temporaryBuild "Finer"

    try {
        New-Item -ItemType Directory -Path $applicationDirectory | Out-Null
        Copy-Item -LiteralPath $executable -Destination (Join-Path $applicationDirectory "Finer.exe")
        Copy-Item -LiteralPath $ffmpeg, $ffprobe -Destination $applicationDirectory
        Copy-Item -LiteralPath (Join-Path $MediaTools "licenses") -Destination $applicationDirectory -Recurse
        Copy-Item -LiteralPath (Join-Path $MediaTools "source") -Destination (Join-Path $applicationDirectory "licenses\source") -Recurse
        Copy-Item -LiteralPath (Join-Path $repositoryRoot "licenses\MEDIA-TOOLS.md") -Destination (Join-Path $applicationDirectory "licenses")
        Invoke-Checked {
            node scripts/dependency-notices.mjs `
                (Join-Path $applicationDirectory "licenses\dependency-notices.txt") `
                $buildTarget
        }
        Copy-Item -LiteralPath (Join-Path $repositoryRoot "README.md") -Destination $applicationDirectory

        $archivePath = Join-Path $outputDirectoryPath "Finer-$version-win-x64-portable.zip"
        Compress-Archive -LiteralPath $applicationDirectory -DestinationPath $archivePath -Force
        Write-Output $archivePath
    } finally {
        if (Test-Path -LiteralPath $temporaryBuild) {
            Remove-Item -LiteralPath $temporaryBuild -Recurse -Force
        }
    }
} finally {
    Pop-Location
}
