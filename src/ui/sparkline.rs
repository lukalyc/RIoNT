//! ASCII sparkline: rolling numeric waveform rendered with block glyphs.

const GLYPHS: [char; 8] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇'];

/// Render `data` into a sparkline of `width` characters.
/// Picks the last `width` samples, normalizes min..max into glyph buckets.
pub fn render(data: &std::collections::VecDeque<f64>, width: usize) -> String {
    if data.is_empty() || width == 0 {
        return String::new();
    }
    let start = data.len().saturating_sub(width);
    let slice: Vec<f64> = data.iter().skip(start).copied().collect();
    let min = slice.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = slice.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;
    let usable = width.saturating_sub(slice.len()); // left pad if fewer samples
    let mut out = String::with_capacity(width * 3);
    for _ in 0..usable {
        out.push(' ');
    }
    for v in &slice {
        let idx = if range <= f64::EPSILON {
            7 // flat line: top block
        } else {
            let norm = (v - min) / range;
            ((norm * 7.0).round() as usize).min(7)
        };
        out.push(GLYPHS[idx]);
    }
    out
}
