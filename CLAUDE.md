# Claude development notes

Finer is a Windows desktop video tool built with Tauri 2, React/TypeScript, and a Rust encoding engine.

## Project map

- `src/`: React interface and TypeScript bindings.
- `engine/`: FFmpeg probing, filter construction, encoding, validation, and MP4 patching.
- `src-tauri/`: Tauri commands and desktop packaging.
- `scripts/`: Windows packaging and media-tool build scripts.
- `docs/`: Technical documentation.

## Product rules

- Keep **Finer** as the TikTok quality workflow.
- Keep **Blender** as the motion workflow. Frame Blend is the first method; Motion Blur is the second.
- Automatic color management converts tagged sRGB footage to Rec.709 and must remain optional.
- Blender exports prioritize quality and use the high-quality two-pass H.264 path.
- Preserve the iOS-inspired desktop design, keyboard access, visible focus states, and light/dark themes.
- Preserve GPL-3.0 attribution and the notices in `NOTICE.md` and `licenses/`.

## Validation

Run these checks before committing:

```powershell
npm run build
cargo check --workspace
cargo test -p finer-engine --lib
```

Build the Windows application with:

```powershell
npm run tauri -- build --no-bundle
```

Do not commit generated directories such as `node_modules/`, `dist/`, `target/`, or `release/`.
