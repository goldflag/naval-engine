//! Survey spikes (radar voids and blunders standing hundreds of metres over their surroundings) replaced with the
//! local median, two passes, as the removed battle maps' bake's `despike` did: a sample is a spike when it stands
//! above the median of its 9 × 9 neighbourhood (edges clamped) by more than 60 m plus the neighbourhood's own spread
//! (its 80th less its 20th percentile). In float32, as the offline bake.
//!
//! [`despike`] gives exactly [`despike_reference`]'s heights, faster: a block whose highest sample stands within 60 m
//! of the lowest within reach of it holds no sample 60 m over its window's minimum, so it is skipped whole (clean
//! coasts are almost all such blocks), and a pass that replaced nothing makes the next pass the same, so it stops.

const REACH: usize = 4;
const SIDE: usize = 2 * REACH + 1;
/// Samples on a side of a block of the quick reject.
const BLOCK: usize = 32;

/// The exact rule, sample by sample: the TypeScript bake's `despike`. Returns the heights and the samples replaced.
pub fn despike_reference(
    heights: &[f32],
    columns: usize,
    rows: usize,
    passes: usize,
) -> (Vec<f32>, usize) {
    let mut h = heights.to_vec();
    let mut row_min = vec![0f32; columns * rows];
    let mut window_min = vec![0f32; columns * rows];
    let mut total = 0;
    for _ in 0..passes {
        for j in 0..rows {
            let base = j * columns;
            for i in 0..columns {
                let mut m = f32::INFINITY;
                for d in -(REACH as isize)..=REACH as isize {
                    let v = h[base + (i as isize + d).clamp(0, columns as isize - 1) as usize];
                    if v < m {
                        m = v;
                    }
                }
                row_min[base + i] = m;
            }
        }
        for j in 0..rows {
            for i in 0..columns {
                let mut m = f32::INFINITY;
                for d in -(REACH as isize)..=REACH as isize {
                    let v = row_min
                        [(j as isize + d).clamp(0, rows as isize - 1) as usize * columns + i];
                    if v < m {
                        m = v;
                    }
                }
                window_min[j * columns + i] = m;
            }
        }
        let mut replaced = h.clone();
        let mut window = [0f32; SIDE * SIDE];
        for j in 0..rows {
            for i in 0..columns {
                if let Some(median) = spike(
                    &h,
                    columns,
                    rows,
                    i,
                    j,
                    window_min[j * columns + i],
                    &mut window,
                ) {
                    replaced[j * columns + i] = median;
                    total += 1;
                }
            }
        }
        h = replaced;
    }
    (h, total)
}

/// Sample (i, j)'s median when it is a spike, given its window's minimum.
#[inline]
fn spike(
    h: &[f32],
    columns: usize,
    rows: usize,
    i: usize,
    j: usize,
    window_min: f32,
    window: &mut [f32; SIDE * SIDE],
) -> Option<f32> {
    let value = h[j * columns + i];
    // The median is at least the minimum, so a sample within 60 m of it is no spike.
    if value - window_min <= 60.0 {
        return None;
    }
    // Nor is one with 41 of its 81 neighbours within 60 m below it or above: the median is then at least value − 60.
    let mut below = 0;
    let mut n = 0;
    let floor = value as f64 - 60.0;
    for dz in -(REACH as isize)..=REACH as isize {
        let row = (j as isize + dz).clamp(0, rows as isize - 1) as usize * columns;
        for dx in -(REACH as isize)..=REACH as isize {
            let v = h[row + (i as isize + dx).clamp(0, columns as isize - 1) as usize];
            window[n] = v;
            n += 1;
            if (v as f64) < floor {
                below += 1;
            }
        }
    }
    if below < 41 {
        return None;
    }
    window.sort_unstable_by(f32::total_cmp);
    let (low, median, high) = (window[16], window[40], window[64]);
    (value - median > 60.0 + (high - low)).then_some(median)
}

/// The same heights as [`despike_reference`], skipping blocks that cannot hold a spike and passes that would change
/// nothing.
pub fn despike(heights: &[f32], columns: usize, rows: usize, passes: usize) -> (Vec<f32>, usize) {
    let mut h = heights.to_vec();
    let mut total = 0;
    let mut window = [0f32; SIDE * SIDE];
    let blocks_x = columns.div_ceil(BLOCK);
    let blocks_y = rows.div_ceil(BLOCK);
    for _ in 0..passes {
        let mut changes: Vec<(usize, f32)> = Vec::new();
        for by in 0..blocks_y {
            let (j0, j1) = (by * BLOCK, ((by + 1) * BLOCK).min(rows));
            for bx in 0..blocks_x {
                let (i0, i1) = (bx * BLOCK, ((bx + 1) * BLOCK).min(columns));
                // The block's highest sample, and the lowest within reach of it.
                let mut high = f32::NEG_INFINITY;
                for j in j0..j1 {
                    for &v in &h[j * columns + i0..j * columns + i1] {
                        high = high.max(v);
                    }
                }
                let (ej0, ej1) = (j0.saturating_sub(REACH), (j1 + REACH).min(rows));
                let (ei0, ei1) = (i0.saturating_sub(REACH), (i1 + REACH).min(columns));
                let mut low = f32::INFINITY;
                for j in ej0..ej1 {
                    for &v in &h[j * columns + ei0..j * columns + ei1] {
                        low = low.min(v);
                    }
                }
                // Every sample's value less its window's minimum is at most this (rounding is monotonic).
                if high - low <= 60.0 {
                    continue;
                }
                for j in j0..j1 {
                    for i in i0..i1 {
                        let mut m = f32::INFINITY;
                        for jj in j.saturating_sub(REACH)..(j + REACH + 1).min(rows) {
                            for &v in &h[jj * columns + i.saturating_sub(REACH)
                                ..jj * columns + (i + REACH + 1).min(columns)]
                            {
                                if v < m {
                                    m = v;
                                }
                            }
                        }
                        if let Some(median) = spike(&h, columns, rows, i, j, m, &mut window) {
                            changes.push((j * columns + i, median));
                        }
                    }
                }
            }
        }
        if changes.is_empty() {
            break;
        }
        total += changes.len();
        for (at, median) in changes {
            h[at] = median;
        }
    }
    (h, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quick_reject_gives_the_reference_heights() {
        let (columns, rows) = (150, 97);
        let mut h: Vec<f32> = (0..columns * rows)
            .map(|k| {
                let (i, j) = ((k % columns) as f32, (k / columns) as f32);
                (i * 0.37).sin() * 40.0 + j * 1.5 - 20.0
            })
            .collect();
        // Spikes alone and in a pair, on flat ground and on a slope, and one beside a cliff that is no spike.
        for (at, v) in [
            (10 * columns + 10, 900.0),
            (50 * columns + 70, 400.0),
            (50 * columns + 71, 380.0),
            (90 * columns + 140, 300.0),
        ] {
            h[at] = v;
        }
        for j in 0..rows {
            for i in 100..columns {
                h[j * columns + i] += 200.0;
            }
        }
        let (want, replaced) = despike_reference(&h, columns, rows, 2);
        let (got, n) = despike(&h, columns, rows, 2);
        assert!(replaced >= 3, "{replaced}");
        assert_eq!(n, replaced);
        assert_eq!(got, want);
        // Clean ground: nothing replaced, nothing changed.
        let clean: Vec<f32> = (0..columns * rows).map(|k| (k % 7) as f32).collect();
        assert_eq!(despike(&clean, columns, rows, 2), (clean.clone(), 0));
    }
}
