# FFmpeg and x264

Finer runs FFmpeg to encode media and FFprobe to read and check the files. The portable Windows release ships both as separate executables. They are built from FFmpeg 7.1.5 with x264 revision `b35605a` and NVIDIA Video Codec headers `n12.1.14.0`. The headers link FFmpeg to the installed NVIDIA driver at runtime.

These builds include GPL components, including libx264. The bundled executables are distributed under the GNU General Public License, version 2 or later. The included licenses explain the terms and the warranty disclaimer.

## What’s in the portable ZIP

- `licenses/` contains the license texts from FFmpeg and x264.
- `licenses/source/` contains the corresponding source archives and the build script. The NVIDIA headers include their license notices in the archive.
- `licenses/media-build.txt` records the exact source revisions, toolchain versions, and build configuration.

To rebuild the tools, run [`scripts/build-media-tools.sh`](../scripts/build-media-tools.sh) from the source tree in MSYS2 UCRT64. If you choose a different FFmpeg build in settings, that build comes with its own license notices.

Upstream project information is available from [FFmpeg](https://ffmpeg.org/legal.html) and [VideoLAN](https://www.videolan.org/developers/x264.html).
