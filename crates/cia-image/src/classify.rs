//! Photo vs Graphic (DESIGN.md 3.4.2), computed on a 512 px thumbnail except
//! alpha, which uses full resolution.

use crate::decode::DecodedImage;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Photo,
    Graphic,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Classification {
    pub class: Class,
    pub has_alpha: bool,
    pub unique_colours: u32,
    pub flat_ratio: f32,
}

pub fn classify(img: &DecodedImage) -> Classification {
    let thumb = crate::resize::thumbnail_rgb(img, 512);
    let (w, h) = (thumb.width() as usize, thumb.height() as usize);
    let px = thumb.as_raw();
    let mut colours: HashSet<u32> = HashSet::new();
    for p in px.chunks_exact(3) {
        if colours.len() <= 65_536 {
            colours.insert((p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32);
        }
    }
    let mut flat = 0u32;
    let mut blocks = 0u32;
    let mut by = 0;
    while by + 8 <= h {
        let mut bx = 0;
        while bx + 8 <= w {
            let (mut lo, mut hi) = (255u8, 0u8);
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    let i = (y * w + x) * 3;
                    let l = ((px[i] as u32 * 299 + px[i + 1] as u32 * 587 + px[i + 2] as u32 * 114)
                        / 1000) as u8;
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
            }
            if hi - lo <= 2 {
                flat += 1;
            }
            blocks += 1;
            bx += 8;
        }
        by += 8;
    }
    let flat_ratio = if blocks == 0 {
        0.0
    } else {
        flat as f32 / blocks as f32
    };
    let unique_colours = colours.len() as u32;
    let class = if unique_colours <= 4096 || flat_ratio >= 0.5 {
        Class::Graphic
    } else {
        Class::Photo
    };
    Classification {
        class,
        has_alpha: img.has_alpha,
        unique_colours,
        flat_ratio,
    }
}
