//! Multi-row ASCII line plot for the matrix zoom view.
//!
//! A single column of data becomes one terminal column; each row is a value
//! band, filled with full/partial block glyphs so curves look smooth at
//! near-zero cost.

const PARTIAL: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Render `data` into `height` rows of `width` columns (row 0 = top).
/// If there are fewer samples than columns, the plot is right-aligned and
/// left-padded; extra samples beyond `width` are dropped (latest wins).
pub fn render(data: &std::collections::VecDeque<f64>, width: usize, height: usize) -> Vec<String> {
    let mut grid = vec![vec![' '; width]; height];
    if width == 0 || height == 0 || data.is_empty() {
        return grid.into_iter().map(|r| r.into_iter().collect()).collect();
    }

    let start = data.len().saturating_sub(width);
    let slice: Vec<f64> = data.iter().skip(start).copied().collect();
    let min = slice.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = slice.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    let pad = width.saturating_sub(slice.len());
    for (i, v) in slice.iter().enumerate() {
        let x = pad + i;
        let norm = if range <= f64::EPSILON { 0.5 } else { (v - min) / range };
        // Row position from the top (0 = top edge of the plot area).
        let pos = (1.0 - norm) * height as f64;
        let whole = (pos as usize).min(height.saturating_sub(1));
        // Everything below the curve point is filled.
        for r in (whole + 1)..height {
            grid[r][x] = '█';
        }
        // The cell containing the point gets a proportional partial fill.
        let frac = (pos - whole as f64).clamp(0.0, 1.0);
        let idx = ((frac * 8.0).round() as usize).clamp(1, 8);
        grid[whole][x] = PARTIAL[idx - 1];
    }

    grid.into_iter().map(|r| r.into_iter().collect()).collect()
}
