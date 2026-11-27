# RIFE motion interpolation

Motion Blur uses an optional runtime in a `rife` folder next to `finer.exe`
(`src-tauri/rife` when building; it is bundled as a resource, not committed):

- `vapoursynth/` – VapourSynth R70 portable with embedded Python 3.12 (LGPL-2.1 / PSF)
- `vapoursynth/vs-plugins/BestSource.dll` – BestSource source filter (MIT)
- `vapoursynth/vs-plugins/vapoursynth-rife-ncnn-vulkan.dll` – RIFE ncnn/Vulkan plugin (MIT)
- `models/rife-v4.26_ensembleFalse` – RIFE v4.26 model (MIT)

No proprietary components (such as SVP) are included. If the folder is missing or
RIFE fails, Finer falls back to its built-in frame blending.
