//! Subband geometry: the 64-subband decomposition (Section 3.7 of the algorithm document).

/// A rectangle of the coefficient array, with its spectral-inversion flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    /// Mirrored along the horizontal axis: the highpass half of a split comes first.
    pub inv_x: bool,
    /// Mirrored along the vertical axis.
    pub inv_y: bool,
}

/// Lengths of the first and second part of a split of length `n`.
#[inline]
pub fn halves(n: usize, inverted: bool) -> (usize, usize) {
    let (lo, hi) = (n.div_ceil(2), n / 2);
    if inverted { (hi, lo) } else { (lo, hi) }
}

impl Region {
    /// Children of a one-level split: top left, top right, bottom left, bottom right.
    pub fn split(&self) -> [Region; 4] {
        let (w1, w2) = halves(self.w, self.inv_x);
        let (h1, h2) = halves(self.h, self.inv_y);
        let (x, y) = (self.x, self.y);
        [
            Region { x, y, w: w1, h: h1, inv_x: false, inv_y: false },
            Region { x: x + w1, y, w: w2, h: h1, inv_x: true, inv_y: false },
            Region { x, y: y + h1, w: w1, h: h2, inv_x: false, inv_y: true },
            Region { x: x + w1, y: y + h1, w: w2, h: h2, inv_x: true, inv_y: true },
        ]
    }

    pub fn len(&self) -> usize {
        self.w * self.h
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
}

const ZORDER: [(usize, usize); 16] = [
    (0, 0), (0, 1), (1, 0), (1, 1), (0, 2), (0, 3), (1, 2), (1, 3),
    (2, 0), (2, 1), (3, 0), (3, 1), (2, 2), (2, 3), (3, 2), (3, 3),
];

/// The decomposition of an image.
#[derive(Debug, Clone)]
pub struct Tree {
    pub width: usize,
    pub height: usize,
    pub subbands: [Region; 64],
    /// Regions to split, parents before children (the order of the forward transform).
    pub splits: Vec<Region>,
    /// Index in `splits` of the split of the diagonal quadrant that holds subbands 60 to 63.
    pub diagonal_split: usize,
}

fn grid(r: Region, splits: &mut Vec<Region>) -> [[Region; 4]; 4] {
    splits.push(r);
    let mut g = [[r; 4]; 4];
    for (q, c) in r.split().into_iter().enumerate() {
        splits.push(c);
        for (t, d) in c.split().into_iter().enumerate() {
            g[2 * (q / 2) + t / 2][2 * (q % 2) + t % 2] = d;
        }
    }
    g
}

impl Tree {
    pub fn new(width: usize, height: usize) -> Tree {
        let mut splits = Vec::with_capacity(21);
        let mut subbands = Vec::with_capacity(64);
        let g = grid(Region { x: 0, y: 0, w: width, h: height, inv_x: false, inv_y: false }, &mut splits);
        let diagonal_split = 4; // image, then quadrants TL, TR, BL, BR
        for (i, j) in ZORDER {
            match (i, j) {
                (0, 0) => {
                    let d = grid(g[0][0], &mut splits);
                    for (i2, j2) in ZORDER {
                        if (i2, j2) == (0, 0) {
                            splits.push(d[0][0]);
                            subbands.extend(d[0][0].split());
                        } else {
                            subbands.push(d[i2][j2]);
                        }
                    }
                }
                (0, 1) | (1, 0) => {
                    let d = grid(g[i][j], &mut splits);
                    subbands.extend(ZORDER.iter().map(|&(a, b)| d[a][b]));
                }
                _ => subbands.push(g[i][j]),
            }
        }
        Tree { width, height, subbands: subbands.try_into().unwrap(), splits, diagonal_split }
    }
}

/// Downsampling factor m_k (area of the image over area of subband k).
pub fn downsampling(k: usize) -> u32 {
    match k {
        0..=3 => 1024,
        4..=50 => 256,
        _ => 16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subbands_tile_the_image() {
        for &(w, h) in &[(32, 32), (1, 1), (5, 3), (17, 31), (416, 389), (1452, 1006), (1600, 1500), (81, 49), (777, 501)] {
            let t = Tree::new(w, h);
            let mut cover = vec![0u8; w * h];
            for r in &t.subbands {
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        cover[y * w + x] += 1;
                    }
                }
            }
            assert!(cover.iter().all(|&c| c == 1), "{w}x{h}");
            assert_eq!(t.splits.len(), 21);
        }
    }

    #[test]
    fn square_layout_matches_the_specification() {
        let t = Tree::new(32, 32);
        let s = &t.subbands;
        assert_eq!((s[0].x, s[0].y, s[0].w), (0, 0, 1));
        assert_eq!((s[4].x, s[4].y, s[4].w), (2, 0, 2));
        assert_eq!((s[19].x, s[19].y, s[19].w), (8, 0, 2));
        assert_eq!((s[51].x, s[51].y, s[51].w), (8, 8, 8));
        assert_eq!((s[52].x, s[52].y), (16, 0));
        assert_eq!((s[63].x, s[63].y), (24, 24));
        let total: f64 = (0..64).map(|k| 1.0 / downsampling(k) as f64).sum();
        assert!((total - 1.0).abs() < 1e-12);
    }

    #[test]
    fn odd_sizes_put_the_short_half_first_when_inverted() {
        // the top-right quadrant of a 7-wide image is mirrored horizontally
        let t = Tree::new(7, 7);
        let tr = t.splits[2];
        assert!(tr.inv_x && !tr.inv_y);
        let c = tr.split();
        assert_eq!((c[0].w, c[1].w), (1, 2)); // width 3: highpass (1) first
    }
}
