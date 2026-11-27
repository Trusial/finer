//! RIFE motion interpolation for Motion Blur.
//!
//! Finer ships a small VapourSynth runtime (VapourSynth, BestSource and the
//! RIFE ncnn/Vulkan plugin with the rife-v4.26 model) in a `rife` folder next to
//! the executable. When it is present, Motion Blur interpolates the recording to
//! twelve times the output rate on the GPU, blends a gamma-weighted window there
//! and pipes only the finished output frames to FFmpeg for encoding.

use crate::{Error, Settings};
use std::path::{Path, PathBuf};

pub(crate) const MODEL: &str = "rife-v4.26_ensembleFalse";

pub(crate) struct Rife {
    pub(crate) vspipe: PathBuf,
    model: PathBuf,
}

impl Rife {
    /// Finds the runtime next to the executable (or in `FINER_RIFE_DIR`).
    pub(crate) fn locate() -> Option<Self> {
        let root = std::env::var_os("FINER_RIFE_DIR").map(PathBuf::from).or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("rife")))
        })?;
        let vspipe = root.join("vapoursynth").join("VSPipe.exe");
        let model = root.join("models").join(MODEL);
        (vspipe.is_file() && model.join("flownet.bin").is_file()).then_some(Self { vspipe, model })
    }

    /// Writes the VapourSynth script for one job and returns its path.
    pub(crate) fn script(
        &self,
        source: &Path,
        source_fps: f64,
        full_range: bool,
        settings: &Settings,
        directory: &Path,
    ) -> Result<PathBuf, Error> {
        let path = directory.join("finer-motion-blur.vpy");
        std::fs::write(&path, script_text(source, &self.model, source_fps, full_range, settings))
            .map_err(|e| Error::io(e, "outputDirectory", "Could not prepare motion blur."))?;
        Ok(path)
    }
}

fn py(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('\\', "/").replace('"', "\\\""))
}

/// Motion Blur, modelled on f0e/blur and improved:
/// * RIFE interpolates to 12x the output rate (720 fps for 60 fps) with scene-cut
///   detection, so cuts never morph into each other;
/// * the window is f0e's `frame gap x blur amount` (5 samples = amount 1), odd and
///   centred, with "ascending" weights that keep the newest frame crisp;
/// * blending happens in 32-bit float with gamma 2 so bright streaks stay bright.
fn script_text(source: &Path, model: &Path, source_fps: f64, full_range: bool, settings: &Settings) -> String {
    let target = settings.target_fps.max(1);
    let interpolated = target * 12;
    let gap = 12;
    let amount = f64::from(settings.blur_samples) / 5.0;
    let mut frames = ((f64::from(gap) * amount).round() as u32).clamp(3, 2 * gap + 1);
    if frames % 2 == 0 {
        frames += 1;
    }
    let frames = frames.min(31);
    let weights = (1..=frames).map(|w| w.to_string()).collect::<Vec<_>>().join(", ");
    let range = if full_range { "full" } else { "limited" };
    let interpolate = if source_fps + 0.5 < f64::from(interpolated) {
        format!(
            "rgb = core.rife.RIFE(rgb, fps_num={interpolated}, fps_den=1, model_path={}, gpu_id=0, sc=True)\n",
            py(model)
        )
    } else {
        format!("rgb = core.std.AssumeFPS(rgb, fpsnum={interpolated}, fpsden=1)\n")
    };
    format!(
        "import vapoursynth as vs\n\
         core = vs.core\n\
         clip = core.bs.VideoSource(source={src}, cachemode=0)\n\
         rgb = core.resize.Bicubic(clip, format=vs.RGBS, matrix_in_s=\"709\", range_in_s=\"{range}\")\n\
         {interpolate}\
         lin = core.std.Expr(rgb, \"x 0 max 2 pow\")\n\
         mixed = core.std.AverageFrames(lin, weights=[{weights}])\n\
         mixed = core.std.Expr(mixed, \"x 0 max 0.5 pow\")\n\
         out = mixed.std.SelectEvery(cycle={gap}, offsets=0)\n\
         out = core.std.AssumeFPS(out, fpsnum={target}, fpsden=1)\n\
         out = core.resize.Bicubic(out, format=vs.YUV420P16, matrix_s=\"709\", range_s=\"limited\")\n\
         out.set_output()\n",
        src = py(source),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderMode;

    #[test]
    fn script_matches_f0e_window_and_interpolates_on_gpu() {
        let settings = Settings {
            render_mode: RenderMode::MotionBlur,
            target_fps: 60,
            blur_samples: 5,
            ..Settings::default()
        };
        let text = script_text(Path::new(r"C:\clips\a b.mp4"), Path::new(r"C:\m"), 240.0, false, &settings);
        assert!(text.contains("fps_num=720"));
        assert!(text.contains("sc=True"));
        assert!(text.contains("weights=[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]"));
        assert!(text.contains("SelectEvery(cycle=12, offsets=0)"));
        assert!(text.contains("\"C:/clips/a b.mp4\""));
    }
}
