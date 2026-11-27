# Finer

Finer is a local Windows video tool with two focused workflows:

- **Finer** prepares H.264/AAC MP4 exports for TikTok and applies the compatible MP4 audio-tail patch.
- **Blender** renders either Premiere-style frame blending or interpolated multi-frame motion blur.

Finer 1.2 adds automatic sRGB-to-Rec.709 color management with a pass-through switch, a higher-quality two-pass Blender export path, and an iOS-inspired desktop interface.

## Getting started

1. Run `Finer-1.2.0-x64-setup.exe` once.
2. Open the Windows Start menu and search for **Finer**.
3. Choose Finer or Blender, then drag in one or more videos.
4. Start the queue. A file called `clip.mp4` becomes `clip_finer.mp4`.

The installer adds Finer to Windows Search and installs FFmpeg and FFprobe with the application. The portable ZIP remains available for use without installation; extract the complete archive before opening `Finer.exe`.

Change video bitrate, GPU acceleration, output location, and advanced options under **Settings**.

See [how the TikTok compatibility patch works](docs/PATCHING.md). Finer is derived from the GPL-3.0 project Upflow; see [NOTICE.md](NOTICE.md).

## Build from source

Install [Rust](https://rustup.rs/) with the MSVC toolchain, [Node.js](https://nodejs.org/) 22.12 or newer, [MSYS2](https://www.msys2.org/), and [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) with **Desktop development with C++** and a Windows SDK.

From the project folder in **MSYS2 UCRT64**, build the media tools. Rebuild these if you’re upgrading from a build without GPU support.

```bash
pacman -S --needed git make tar gzip mingw-w64-ucrt-x86_64-gcc mingw-w64-ucrt-x86_64-nasm mingw-w64-ucrt-x86_64-pkgconf
bash scripts/build-media-tools.sh
```

Then switch to **PowerShell** in the same project folder.

```powershell
rustup default stable-x86_64-pc-windows-msvc
.\scripts\package-windows.ps1
```

The portable ZIP lands in `release\`. See the [media tools notice](licenses/MEDIA-TOOLS.md) for the bundled FFmpeg and x264 sources and licenses.

To create the searchable Windows installer from existing media tools:

```powershell
npm run build:installer -- -MediaTools .\media-tools\bin
```

The NSIS setup executable lands in `target\release\bundle\nsis\`.
