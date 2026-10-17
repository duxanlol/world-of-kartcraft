//! Kart paints and boost trails: the cosmetics a racer earns on a Kartcraft server (its
//! `kartcraft_cosmetic`, `Kartcraft.cpp` "Cosmetics"), by index into the server's catalogues.
//! The server names them and says who has earned which; here is how each looks. A paint repaints
//! a kart's coloured parts in its colour at their own brightness, its greys and blacks (metal,
//! tyres, the engine) left as they are; a trail is a ribbon of light behind a kart at speed.

use std::sync::atomic::{AtomicU8, Ordering};

/// A paint: its colour (sRGB) and how bright it lays on, against the part it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub colour: [f32; 3],
    pub shade: f32,
}

/// The server's PAINTS by index; 0, stock, repaints nothing.
pub const PAINTS: [Option<Paint>; 9] = [
    None,
    // Copper
    Some(Paint {
        colour: [0.85, 0.45, 0.25],
        shade: 1.0,
    }),
    // Silver
    Some(Paint {
        colour: [0.80, 0.82, 0.86],
        shade: 1.15,
    }),
    // Gold
    Some(Paint {
        colour: [1.0, 0.78, 0.2],
        shade: 1.2,
    }),
    // Midnight
    Some(Paint {
        colour: [0.15, 0.18, 0.45],
        shade: 0.6,
    }),
    // Record Red
    Some(Paint {
        colour: [0.9, 0.1, 0.1],
        shade: 1.0,
    }),
    // Challenger Teal
    Some(Paint {
        colour: [0.1, 0.8, 0.75],
        shade: 1.0,
    }),
    // Champion Violet
    Some(Paint {
        colour: [0.6, 0.2, 0.95],
        shade: 1.05,
    }),
    // Astral: found only in an Astral Cache
    Some(Paint {
        colour: [0.35, 0.55, 1.0],
        shade: 1.3,
    }),
];

/// The server's TRAILS by index: a colour (sRGB), or the rainbow; 0, none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trail {
    Colour([f32; 3]),
    Rainbow,
}

pub const TRAILS: [Option<Trail>; 7] = [
    None,
    // Sparks
    Some(Trail::Colour([1.0, 0.85, 0.3])),
    // Wumpa
    Some(Trail::Colour([1.0, 0.45, 0.1])),
    // Frost
    Some(Trail::Colour([0.5, 0.9, 1.0])),
    // Rainbow
    Some(Trail::Rainbow),
    // Stardust and Comet: found only in an Astral Cache
    Some(Trail::Colour([0.85, 0.7, 1.0])),
    Some(Trail::Colour([0.75, 1.0, 0.95])),
];

pub fn paint(index: u8) -> Option<Paint> {
    PAINTS.get(usize::from(index)).copied().flatten()
}

pub fn trail(index: u8) -> Option<Trail> {
    TRAILS.get(usize::from(index)).copied().flatten()
}

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// One colour (sRGB, 0 to 1) under `paint`: a coloured one becomes the paint at its own
/// brightness, a grey stays grey, and those between go part of the way.
pub fn repaint(c: [f32; 3], paint: Paint) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let saturation = if max > 0.0 { (max - min) / max } else { 0.0 };
    let k = ((saturation - 0.15) / 0.3).clamp(0.0, 1.0);
    let k = k * k * (3.0 - 2.0 * k);
    let scale = luma(c) / luma(paint.colour).max(0.05) * paint.shade;
    std::array::from_fn(|i| {
        let painted = (paint.colour[i] * scale).min(1.0);
        c[i] + (painted - c[i]) * k
    })
}

/// A trail's colour `t` seconds into its ribbon's life (the rainbow runs along it).
pub fn trail_colour(trail: Trail, t: f32) -> [f32; 3] {
    match trail {
        Trail::Colour(c) => c,
        Trail::Rainbow => {
            let h = (t * 1.5).rem_euclid(1.0) * 6.0;
            let x = 1.0 - (h % 2.0 - 1.0).abs();
            match h as u32 {
                0 => [1.0, x, 0.0],
                1 => [x, 1.0, 0.0],
                2 => [0.0, 1.0, x],
                3 => [0.0, x, 1.0],
                4 => [x, 0.0, 1.0],
                _ => [1.0, 0.0, x],
            }
        }
    }
}

static OUR_PAINT: AtomicU8 = AtomicU8::new(0);
static OUR_TRAIL: AtomicU8 = AtomicU8::new(0);

/// What we wear, as the server last said (`KartPaint`, `KartTrail`); an index past the
/// catalogue is none.
pub fn set_ours(paint: u8, trail: u8) {
    let paint = if self::paint(paint).is_some() {
        paint
    } else {
        0
    };
    let trail = if self::trail(trail).is_some() {
        trail
    } else {
        0
    };
    OUR_PAINT.store(paint, Ordering::Relaxed);
    OUR_TRAIL.store(trail, Ordering::Relaxed);
}

/// Our (paint, trail).
pub fn ours() -> (u8, u8) {
    (
        OUR_PAINT.load(Ordering::Relaxed),
        OUR_TRAIL.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paint_recolours_the_coloured_parts_and_keeps_the_greys() {
        let gold = paint(3).unwrap();
        // A saturated blue body becomes gold at about its brightness.
        let blue = [0.1, 0.2, 0.9];
        let out = repaint(blue, gold);
        assert!(out[0] > out[2] && out[1] > out[2], "{out:?}");
        assert!(
            (luma(out) - luma(blue) * gold.shade).abs() < 0.08,
            "{out:?}"
        );
        // Grey metal and black tyres stay as they are.
        for grey in [[0.5, 0.5, 0.5], [0.05, 0.05, 0.05], [0.0, 0.0, 0.0]] {
            assert_eq!(repaint(grey, gold), grey);
        }
        // Stock, and an index past the catalogue, paint nothing.
        assert!(paint(0).is_none() && paint(200).is_none());
    }

    #[test]
    fn our_wear_drops_what_the_catalogue_lacks() {
        set_ours(3, 4);
        assert_eq!(ours(), (3, 4));
        set_ours(99, 99);
        assert_eq!(ours(), (0, 0));
    }

    #[test]
    fn the_rainbow_runs_through_the_hues() {
        let c = |t| trail_colour(Trail::Rainbow, t);
        assert_eq!(c(0.0), [1.0, 0.0, 0.0]);
        assert_ne!(c(0.2), c(0.0));
        assert_eq!(
            trail_colour(Trail::Colour([0.1, 0.2, 0.3]), 5.0),
            [0.1, 0.2, 0.3]
        );
    }
}
