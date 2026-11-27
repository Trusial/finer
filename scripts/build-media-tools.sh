#!/usr/bin/env bash
# Run from an MSYS2 UCRT64 shell. The extracted source archives are the build inputs.
set -euo pipefail

if [[ ${MSYSTEM:-} != UCRT64 ]]; then
  echo 'Run this from the MSYS2 UCRT64 shell.' >&2
  exit 1
fi

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
output_directory="$repository_root/media-tools"
work_directory="$(mktemp -d)"
trap 'rm -rf "$work_directory"' EXIT

stage_directory="$work_directory/stage"
install_prefix="$work_directory/prefix"
jobs="${UPFLOW_BUILD_JOBS:-4}"
ffmpeg_tag=n7.1.5
ffmpeg_tag_object=eefb3654c445a9004712036be4ffb01a68b2c3e1
x264_commit=b35605ace3ddf7c1a5d67a2eb553f034aef41d55
nv_headers_tag=n12.1.14.0
nv_headers_commit=1889e62e2d35ff7aa9baca2bceb14f053785e6f1

for tool in git gcc make nasm pkg-config tar gzip objdump; do
  command -v "$tool" >/dev/null
done

mkdir -p \
  "$stage_directory/bin" \
  "$stage_directory/licenses" \
  "$stage_directory/source" \
  "$work_directory/ffmpeg" \
  "$work_directory/x264" \
  "$work_directory/nv-codec-headers"

git init -q "$work_directory/ffmpeg-git"
git -C "$work_directory/ffmpeg-git" fetch \
  --depth=1 \
  https://github.com/FFmpeg/FFmpeg.git \
  "refs/tags/$ffmpeg_tag"
[[ "$(git -C "$work_directory/ffmpeg-git" rev-parse FETCH_HEAD)" == "$ffmpeg_tag_object" ]]
git -C "$work_directory/ffmpeg-git" archive --format=tar "FETCH_HEAD^{commit}" \
  | gzip -n > "$stage_directory/source/ffmpeg-$ffmpeg_tag.tar.gz"
tar -xzf "$stage_directory/source/ffmpeg-$ffmpeg_tag.tar.gz" -C "$work_directory/ffmpeg"

git init -q "$work_directory/x264-git"
x264_url=https://code.videolan.org/videolan/x264.git
if ! git -C "$work_directory/x264-git" fetch --depth=1 "$x264_url" "$x264_commit"; then
  git -C "$work_directory/x264-git" fetch --no-tags "$x264_url" refs/heads/stable
fi
git -C "$work_directory/x264-git" archive --format=tar "$x264_commit" \
  | gzip -n > "$stage_directory/source/x264-$x264_commit.tar.gz"
tar -xzf "$stage_directory/source/x264-$x264_commit.tar.gz" -C "$work_directory/x264"

git init -q "$work_directory/nv-codec-headers-git"
git -C "$work_directory/nv-codec-headers-git" fetch \
  --depth=1 https://github.com/FFmpeg/nv-codec-headers.git "refs/tags/$nv_headers_tag"
[[ "$(git -C "$work_directory/nv-codec-headers-git" rev-parse 'FETCH_HEAD^{commit}')" == "$nv_headers_commit" ]]
git -C "$work_directory/nv-codec-headers-git" archive --format=tar "$nv_headers_commit" \
  | gzip -n > "$stage_directory/source/nv-codec-headers-$nv_headers_tag.tar.gz"
tar -xzf "$stage_directory/source/nv-codec-headers-$nv_headers_tag.tar.gz" -C "$work_directory/nv-codec-headers"
make -C "$work_directory/nv-codec-headers" PREFIX="$install_prefix" install

(
  cd "$work_directory/x264"
  ./configure \
    --host=x86_64-w64-mingw32 \
    --prefix="$install_prefix" \
    --enable-static \
    --disable-cli
  make -j"$jobs"
  make install
)

(
  cd "$work_directory/ffmpeg"
  export PKG_CONFIG_PATH="$install_prefix/lib/pkgconfig"
  ./configure \
    --prefix="$install_prefix" \
    --target-os=mingw32 \
    --arch=x86_64 \
    --enable-gpl \
    --enable-libx264 \
    --disable-autodetect \
    --enable-ffnvcodec \
    --enable-nvdec \
    --enable-nvenc \
    --enable-d3d11va \
    --enable-mediafoundation \
    --disable-network \
    --disable-shared \
    --enable-static \
    --disable-doc \
    --disable-debug \
    --disable-ffplay \
    --pkg-config-flags=--static \
    --extra-ldflags=-static
  make -j"$jobs" ffmpeg.exe ffprobe.exe
  cp ffmpeg.exe ffprobe.exe "$stage_directory/bin/"
)

for tool in "$stage_directory/bin/"*.exe; do
  objdump -f "$tool" | awk '/file format pei-x86-64/{found=1} END {exit !found}'
  # The portable distribution relies only on DLLs supplied with Windows.
  if objdump -p "$tool" | awk '/DLL Name:/{print tolower($NF)}' | grep -Eq '^lib(winpthread|gcc|stdc\+\+)'; then
    echo "Unexpected MinGW runtime dependency in $(basename "$tool")." >&2
    exit 1
  fi
done

cp "$work_directory/ffmpeg"/COPYING.* "$stage_directory/licenses/"
cp "$work_directory/x264/COPYING" "$stage_directory/licenses/x264-GPL-2.0.txt"
# The header files carry their license notices and are included in full in the source archive.
cp "$repository_root/scripts/build-media-tools.sh" "$stage_directory/source/"
{
  printf 'FFmpeg tag: %s (%s)\nx264 commit: %s\nnv-codec-headers tag: %s (%s)\n\n' \
    "$ffmpeg_tag" "$ffmpeg_tag_object" "$x264_commit" "$nv_headers_tag" "$nv_headers_commit"
  gcc --version
  nasm -v
  "$stage_directory/bin/ffmpeg.exe" -version
  "$stage_directory/bin/ffprobe.exe" -version
} > "$stage_directory/licenses/media-build.txt"

# Check support without assuming the build machine has a GPU.
encoders="$("$stage_directory/bin/ffmpeg.exe" -hide_banner -encoders)"
awk '$2 == "h264_nvenc" { nv = 1 } $2 == "h264_mf" { mf = 1 } END { exit !(nv && mf) }' <<< "$encoders"
"$stage_directory/bin/ffmpeg.exe" -hide_banner -hwaccels | awk '/^d3d11va[[:space:]]*$/ { found = 1 } END { exit !found }'
"$stage_directory/bin/ffmpeg.exe" -hide_banner -h decoder=hevc \
  | awk '/Supported hardware devices:.*[[:space:]]cuda([[:space:]]|$)/ { found = 1 } END { exit !found }'

# Check that the compiled encoder is present and can actually produce frames.
"$stage_directory/bin/ffmpeg.exe" -v error -f lavfi -i 'testsrc2=s=64x64:r=25:d=1' \
  -c:v libx264 -f mp4 "$work_directory/check.mp4"
[[ "$("$stage_directory/bin/ffprobe.exe" -v error -select_streams v:0 -show_entries stream=codec_name -of csv=p=0 "$work_directory/check.mp4" | tr -d '\r')" == h264 ]]

mkdir -p "$output_directory"
cp -R \
  "$stage_directory/bin" \
  "$stage_directory/licenses" \
  "$stage_directory/source" \
  "$output_directory/"
printf 'Windows media tools created at %s\n' "$output_directory"
