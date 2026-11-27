//! Premiere's handling of OBS recordings, measured from a real Premiere export.
//!
//! OBS tags its files with the sRGB transfer curve. Premiere lifts the shadows
//! of such footage (obsidian and other dark blocks get visibly brighter) while
//! leaving highlights almost untouched. The curve below was measured pixel by
//! pixel against a Premiere export of the same recording and is applied per RGB
//! channel (the three channels matched within two code values).

use std::path::PathBuf;

const PREMIERE_LOOK: [f32; 256] = [
    0.00000, 0.03326, 0.04125, 0.04967, 0.05839, 0.06739, 0.07219, 0.07625,
    0.08046, 0.08482, 0.08903, 0.09237, 0.09586, 0.09949, 0.10327, 0.10690,
    0.11097, 0.11445, 0.11794, 0.12171, 0.12593, 0.12956, 0.13348, 0.13740,
    0.14132, 0.14510, 0.14902, 0.15294, 0.15686, 0.16064, 0.16485, 0.16718,
    0.17081, 0.17473, 0.17836, 0.18199, 0.18548, 0.18896, 0.19245, 0.19593,
    0.20102, 0.20479, 0.20828, 0.21191, 0.21569, 0.21961, 0.22353, 0.22731,
    0.23123, 0.23515, 0.23907, 0.24285, 0.24662, 0.25054, 0.25418, 0.25781,
    0.26144, 0.26492, 0.26855, 0.27219, 0.27596, 0.27959, 0.28308, 0.28686,
    0.29063, 0.29455, 0.29847, 0.30225, 0.30603, 0.30966, 0.31329, 0.31692,
    0.32041, 0.32389, 0.32738, 0.33086, 0.33450, 0.33798, 0.34161, 0.34539,
    0.34916, 0.35294, 0.35643, 0.36006, 0.36383, 0.36732, 0.37095, 0.37458,
    0.37836, 0.38228, 0.38635, 0.39056, 0.39434, 0.39797, 0.40174, 0.40552,
    0.40915, 0.41278, 0.41627, 0.41932, 0.42280, 0.42643, 0.42992, 0.43355,
    0.43733, 0.44110, 0.44459, 0.44822, 0.45229, 0.45592, 0.45955, 0.46304,
    0.46681, 0.47044, 0.47422, 0.47814, 0.48192, 0.48569, 0.48962, 0.49368,
    0.49775, 0.50153, 0.50501, 0.50893, 0.51256, 0.51605, 0.51968, 0.52317,
    0.52651, 0.53028, 0.53435, 0.53842, 0.54190, 0.54568, 0.54960, 0.55338,
    0.55744, 0.56122, 0.56500, 0.56805, 0.57139, 0.57487, 0.57836, 0.58199,
    0.58548, 0.58882, 0.59274, 0.59666, 0.60058, 0.60465, 0.60857, 0.61249,
    0.61627, 0.62019, 0.62426, 0.62803, 0.63152, 0.63500, 0.63849, 0.64212,
    0.64575, 0.64924, 0.65287, 0.65635, 0.65970, 0.66333, 0.66710, 0.67088,
    0.67451, 0.67829, 0.68221, 0.68540, 0.68874, 0.69223, 0.69542, 0.69891,
    0.70254, 0.70617, 0.70937, 0.71271, 0.71692, 0.72070, 0.72433, 0.72810,
    0.73159, 0.73479, 0.73827, 0.74234, 0.74597, 0.74873, 0.75251, 0.75657,
    0.76064, 0.76471, 0.76877, 0.77226, 0.77560, 0.77879, 0.78330, 0.78606,
    0.78853, 0.79070, 0.79332, 0.79593, 0.79869, 0.80087, 0.80320, 0.80552,
    0.80944, 0.81438, 0.81975, 0.82484, 0.82977, 0.83515, 0.84081, 0.84662,
    0.85156, 0.85592, 0.85955, 0.86318, 0.86667, 0.87015, 0.87335, 0.87727,
    0.88105, 0.88488, 0.88872, 0.89256, 0.89639, 0.90023, 0.90407, 0.90791,
    0.91174, 0.91558, 0.91942, 0.92326, 0.92709, 0.93093, 0.93477, 0.93860,
    0.94244, 0.94628, 0.95012, 0.95395, 0.95779, 0.96163, 0.96546, 0.96930,
    0.97314, 0.97698, 0.98081, 0.98465, 0.98849, 0.99233, 0.99616, 1.00000,
];

/// Writes the curve as a 1D .cube LUT (once) and returns an FFmpeg filter chain
/// that applies it at 16-bit precision and returns to Rec.709 limited range.
pub(crate) fn premiere_look_filter() -> String {
    let path = lut_path();
    format!(
        "scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=gbrp16le,lut1d=file='{}':interp=cubic,scale=out_color_matrix=bt709:out_range=tv,format=yuv420p,setparams=color_trc=bt709:color_primaries=bt709:colorspace=bt709",
        escape(&path)
    )
}

fn cube() -> String {
    let mut text = String::from("TITLE \"Finer Premiere look\"\nLUT_1D_SIZE 256\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n");
    for value in PREMIERE_LOOK {
        text.push_str(&format!("{value:.5} {value:.5} {value:.5}\n"));
    }
    text
}

fn lut_path() -> PathBuf {
    let path = std::env::temp_dir().join("finer-premiere-look-v1.cube");
    let text = cube();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        let _ = std::fs::write(&path, &text);
    }
    path
}

/// FFmpeg filter-graph escaping for a path inside single quotes.
fn escape(path: &std::path::Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "/")
        .replace(':', "\\:")
        .replace('\'', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_lifts_shadows_and_keeps_black_and_white() {
        assert_eq!(PREMIERE_LOOK[0], 0.0);
        assert!((PREMIERE_LOOK[255] - 1.0).abs() < 1e-4);
        assert!(PREMIERE_LOOK.windows(2).all(|w| w[1] >= w[0]));
        assert!(PREMIERE_LOOK[36] * 255.0 > 40.0);
        assert!(premiere_look_filter().contains("lut1d=file="));
    }
}
