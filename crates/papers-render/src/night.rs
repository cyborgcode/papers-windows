//! Night mode: recolouring rendered pages for reading in the dark.
//!
//! The filter runs on the render thread right after PDFium has drawn a tile, so
//! the UI thread never touches pixels. It is a per-pixel affine transform with a
//! lookup-table fast path for grey pixels, which are the vast majority on text
//! pages, and it is spread over all cores with rayon.
//!
//! The "smart" style is `invert` followed by a 180° hue rotation (the classic
//! "dark reader" filter): black and white swap, but red stays red and blue stays
//! blue, so highlights, links and figures keep their meaning. On grey pixels the
//! hue-rotation matrix is the identity (each row sums to one), so text is a plain
//! inversion. The result is then compressed into a softer range so white paper
//! becomes a dark grey instead of pure black, which is easier on the eyes.

use egui::Color32;
use rayon::prelude::*;

/// How pages are recoloured in night mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum NightStyle {
    /// Invert lightness and keep hues, like GNOME Papers.
    #[default]
    Smart,
    /// Plain colour inversion.
    Invert,
}

impl NightStyle {
    pub const ALL: [NightStyle; 2] = [NightStyle::Smart, NightStyle::Invert];

    pub fn label(self) -> &'static str {
        match self {
            NightStyle::Smart => "Keep colours",
            NightStyle::Invert => "Invert colours",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            NightStyle::Smart => "smart",
            NightStyle::Invert => "invert",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }

    /// The colour that white paper turns into, used to paint pages that have not been rendered yet.
    pub fn paper(self) -> Color32 {
        let v = Filter::new(self).range[0];
        Color32::from_rgb(v, v, v)
    }
}

/// Fixed-point (×1024) matrix rows; every row sums to 1024 so greys are unchanged.
const HUE_ROTATE_180: [[i32; 3]; 3] = [[-588, 1465, 147], [436, 441, 147], [436, 1465, -877]];
const IDENTITY: [[i32; 3]; 3] = [[1024, 0, 0], [0, 1024, 0], [0, 0, 1024]];

/// A ready-to-apply night filter.
#[derive(Clone)]
pub struct Filter {
    matrix: [[i32; 3]; 3],
    /// Maps an inverted channel value (0 = was white) to the output value.
    range: [u8; 256],
}

impl Filter {
    pub fn new(style: NightStyle) -> Self {
        let (matrix, lo, hi) = match style {
            // Exactly Papers' own `invert(1) hue-rotate(180deg)`: white paper → black.
            NightStyle::Smart => (HUE_ROTATE_180, 0.0_f32, 255.0_f32),
            NightStyle::Invert => (IDENTITY, 0.0, 255.0),
        };
        let mut range = [0u8; 256];
        for (i, v) in range.iter_mut().enumerate() {
            *v = (lo + (hi - lo) * i as f32 / 255.0).round() as u8;
        }
        Filter { matrix, range }
    }

    #[inline(always)]
    fn pixel(&self, c: Color32) -> Color32 {
        let (r, g, b, a) = (c.r(), c.g(), c.b(), c.a());
        if r == g && g == b {
            let v = self.range[255 - r as usize];
            return Color32::from_rgba_premultiplied(v, v, v, a);
        }
        let ir = 255 - r as i32;
        let ig = 255 - g as i32;
        let ib = 255 - b as i32;
        let m = &self.matrix;
        let ch = |row: &[i32; 3]| -> u8 {
            let v = (row[0] * ir + row[1] * ig + row[2] * ib + 512) >> 10;
            self.range[v.clamp(0, 255) as usize]
        };
        Color32::from_rgba_premultiplied(ch(&m[0]), ch(&m[1]), ch(&m[2]), a)
    }

    /// Recolour a buffer in place, in parallel.
    pub fn apply(&self, pixels: &mut [Color32]) {
        const CHUNK: usize = 32 * 1024;
        if pixels.len() <= CHUNK {
            pixels.iter_mut().for_each(|p| *p = self.pixel(*p));
        } else {
            pixels
                .par_chunks_mut(CHUNK)
                .for_each(|chunk| chunk.iter_mut().for_each(|p| *p = self.pixel(*p)));
        }
    }

    /// Recolour `src` into a new buffer, in parallel (avoids a separate copy pass).
    pub fn apply_copy(&self, src: &[Color32]) -> Vec<Color32> {
        let mut out = Vec::with_capacity(src.len());
        src.par_iter().map(|p| self.pixel(*p)).collect_into_vec(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_is_exact() {
        let f = Filter::new(NightStyle::Invert);
        let mut px = vec![
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(0, 0, 0),
            Color32::from_rgb(10, 200, 30),
        ];
        f.apply(&mut px);
        assert_eq!(px[0], Color32::from_rgb(0, 0, 0));
        assert_eq!(px[1], Color32::from_rgb(255, 255, 255));
        assert_eq!(px[2], Color32::from_rgb(245, 55, 225));
    }

    #[test]
    fn smart_greys_invert_like_papers() {
        let f = Filter::new(NightStyle::Smart);
        let mut px = vec![Color32::WHITE, Color32::BLACK, Color32::from_gray(128)];
        f.apply(&mut px);
        assert_eq!(px[0], Color32::BLACK);
        assert_eq!(px[1], Color32::WHITE);
        assert!(px[0].r() == px[0].g() && px[0].g() == px[0].b());
        // The matrix path agrees with the grey fast path.
        let slow = {
            let ir = 255 - 128;
            let v = (HUE_ROTATE_180[0].iter().sum::<i32>() * ir + 512) >> 10;
            f.range[v as usize]
        };
        assert_eq!(px[2].r(), slow);
    }

    #[test]
    fn smart_keeps_hue_family() {
        let f = Filter::new(NightStyle::Smart);
        // A pure red highlight stays reddish (red channel dominant).
        let mut px = vec![Color32::from_rgb(220, 30, 30), Color32::from_rgb(30, 30, 220)];
        f.apply(&mut px);
        assert!(px[0].r() > px[0].g() && px[0].r() > px[0].b(), "{:?}", px[0]);
        assert!(px[1].b() > px[1].r() && px[1].b() > px[1].g(), "{:?}", px[1]);
    }

    #[test]
    fn parallel_matches_serial() {
        let f = Filter::new(NightStyle::Smart);
        let src: Vec<Color32> = (0..200_000u32)
            .map(|i| Color32::from_rgb((i % 251) as u8, (i % 241) as u8, (i % 239) as u8))
            .collect();
        let mut a = src.clone();
        f.apply(&mut a);
        let b = f.apply_copy(&src);
        let c: Vec<Color32> = src.iter().map(|p| f.pixel(*p)).collect();
        assert_eq!(a, c);
        assert_eq!(b, c);
    }
}
