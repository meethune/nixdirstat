//! Colorblind-safe palette and colour utilities for the treemap.
//!
//! Provides an Okabe-Ito categorical colour palette, proportional darkening,
//! contrast-aware text colour selection, and `NO_COLOR` environment support.
//!
//! # Functions
//!
//! - [`category_color`] — Okabe-Ito RGB mapping for [`FileCategory`]
//! - [`darken`] — proportional RGB darkening
//! - [`contrast_text_color`] — black or white for readable text on any background
//! - [`no_color_fallback`] — evenly-spaced grayscale for `NO_COLOR` terminals
//! - [`is_color_enabled`] — checks the `NO_COLOR` environment variable

use ratatui::style::Color;

use crate::types::FileCategory;

/// Return the Okabe-Ito RGB colour for the given [`FileCategory`].
///
/// The [Okabe-Ito palette](https://jfly.uni-koeln.de/color/) is safe for
/// protanopia, deuteranopia, and tritanopia. The mapping is semantic: every
/// file of the same category shares a colour regardless of extension.
/// `NoExtension` and `Other` use distinct grays as they have no semantic meaning.
pub const fn category_color(cat: FileCategory) -> Color {
    match cat {
        FileCategory::Code => Color::Rgb(0, 114, 178), // Blue       #0072B2
        FileCategory::Document => Color::Rgb(86, 180, 233), // Sky Blue   #56B4E9
        FileCategory::Image => Color::Rgb(230, 159, 0), // Orange     #E69F00
        FileCategory::Audio => Color::Rgb(213, 94, 0), // Vermillion #D55E00
        FileCategory::Video => Color::Rgb(204, 121, 167), // Red-Purple #CC79A7
        FileCategory::Archive => Color::Rgb(0, 158, 115), // Blue-Green #009E73
        FileCategory::Binary => Color::Rgb(240, 228, 66), // Yellow     #F0E442
        FileCategory::NoExtension => Color::Rgb(120, 120, 120), // Dark Gray  #787878
        FileCategory::Other => Color::Rgb(170, 170, 170), // Light Gray #AAAAAA
    }
}

/// Proportionally darken an RGB colour by reducing each channel by `amount`.
///
/// `amount` is a fraction in `[0.0, 1.0]`:
/// - `0.0` leaves the colour unchanged.
/// - `1.0` produces black (`Color::Rgb(0, 0, 0)`).
///
/// Non-RGB colour variants are returned unchanged.
pub fn darken(color: Color, amount: f32) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let factor = 1.0_f32 - amount;
    // `factor` ∈ [0.0, 1.0] and each channel ∈ [0, 255], so the rounded
    // product is in [0.0, 255.0] — the casts below are safe:
    // - cast_sign_loss: false positive — the product is non-negative.
    // - cast_possible_truncation: false positive — the product is ≤ 255.0.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let dr = (f32::from(r) * factor).round() as u8;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let dg = (f32::from(g) * factor).round() as u8;
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let db = (f32::from(b) * factor).round() as u8;
    Color::Rgb(dr, dg, db)
}

/// Choose black or white text for readability on a given background colour.
///
/// Uses the ITU-R BT.601 luminance formula `L = 0.299·R + 0.587·G + 0.114·B`.
/// Returns [`Color::Black`] when `L > 128` (light background) and
/// [`Color::White`] otherwise (dark background). Non-RGB colours default to
/// [`Color::White`].
pub fn contrast_text_color(bg: Color) -> Color {
    let Color::Rgb(r, g, b) = bg else {
        return Color::White;
    };
    let luminance = 0.114_f32.mul_add(
        f32::from(b),
        0.587_f32.mul_add(f32::from(g), 0.299_f32 * f32::from(r)),
    );
    if luminance > 128.0_f32 {
        Color::Black
    } else {
        Color::White
    }
}

/// Return an evenly-spaced grayscale colour for a [`FileCategory`].
///
/// Maps the 9 variants to evenly-spaced gray values in `[40, 220]`, producing
/// visually distinct shades suitable for `NO_COLOR` terminals where RGB colour
/// is unavailable.
pub const fn no_color_fallback(cat: FileCategory) -> Color {
    // Evenly spaced from 40 to 220 (step ≈ 22.5, alternating 22/23).
    let v: u8 = match cat {
        FileCategory::Code => 40,
        FileCategory::Image => 62,
        FileCategory::Document => 85,
        FileCategory::Archive => 107,
        FileCategory::Audio => 130,
        FileCategory::Video => 152,
        FileCategory::Binary => 175,
        FileCategory::NoExtension => 197,
        FileCategory::Other => 220,
    };
    Color::Rgb(v, v, v)
}

/// Return `true` if colour output is allowed by the environment.
///
/// Returns `false` when the `NO_COLOR` environment variable is set to any
/// value (including empty string), following the
/// [NO_COLOR specification](https://no-color.org/).
pub fn is_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ratatui::style::Color;

    use super::*;
    use crate::types::FileCategory;

    // --- category_color ---

    #[test]
    fn category_color_code_is_okabe_ito_blue() {
        assert_eq!(category_color(FileCategory::Code), Color::Rgb(0, 114, 178));
    }

    #[test]
    fn category_color_all_variants_are_distinct() {
        let colors: Vec<Color> = [
            FileCategory::Code,
            FileCategory::Image,
            FileCategory::Document,
            FileCategory::Archive,
            FileCategory::Audio,
            FileCategory::Video,
            FileCategory::Binary,
            FileCategory::NoExtension,
            FileCategory::Other,
        ]
        .iter()
        .copied()
        .map(category_color)
        .collect();
        let unique: std::collections::HashSet<_> = colors.iter().collect();
        assert_eq!(unique.len(), colors.len());
    }

    // --- darken ---

    #[test]
    fn darken_zero_is_identity() {
        let c = Color::Rgb(200, 100, 50);
        assert_eq!(darken(c, 0.0), c);
    }

    #[test]
    fn darken_one_is_black() {
        assert_eq!(darken(Color::Rgb(200, 100, 50), 1.0), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn darken_thirty_percent() {
        assert_eq!(
            darken(Color::Rgb(200, 100, 50), 0.3),
            Color::Rgb(140, 70, 35)
        );
    }

    #[test]
    fn darken_non_rgb_returns_unchanged() {
        assert_eq!(darken(Color::Gray, 0.5), Color::Gray);
    }

    // --- contrast_text_color ---

    #[test]
    fn contrast_white_on_dark_blue() {
        assert_eq!(contrast_text_color(Color::Rgb(0, 114, 178)), Color::White);
    }

    #[test]
    fn contrast_black_on_yellow() {
        assert_eq!(contrast_text_color(Color::Rgb(240, 228, 66)), Color::Black);
    }

    // --- no_color_fallback ---

    #[test]
    fn no_color_fallback_all_distinct() {
        let grays: Vec<Color> = [
            FileCategory::Code,
            FileCategory::Image,
            FileCategory::Document,
            FileCategory::Archive,
            FileCategory::Audio,
            FileCategory::Video,
            FileCategory::Binary,
            FileCategory::NoExtension,
            FileCategory::Other,
        ]
        .iter()
        .copied()
        .map(no_color_fallback)
        .collect();
        let unique: std::collections::HashSet<_> = grays.iter().collect();
        assert_eq!(unique.len(), grays.len());
        // All grayscale values must be Rgb(v, v, v) with v ∈ [40, 220].
        for color in &grays {
            assert!(
                matches!(color, Color::Rgb(..)),
                "expected Color::Rgb, got {color:?}"
            );
            if let Color::Rgb(r, g, b) = color {
                assert_eq!(r, g, "grayscale: r == g");
                assert_eq!(g, b, "grayscale: g == b");
                assert!(*r >= 40, "value {r} below minimum 40");
                assert!(*r <= 220, "value {r} above maximum 220");
            }
        }
    }

    // --- is_color_enabled ---

    #[test]
    fn is_color_enabled_consistent_with_no_color_env() {
        // std::env::set_var and remove_var are unsafe in Rust 1.83+, and
        // `unsafe_code = "forbid"` prevents their use even in test code.
        // We therefore verify that is_color_enabled() is consistent with the
        // current environment state without mutating it.  The test is valid in
        // both CI (NO_COLOR absent → returns true) and NO_COLOR=1 runs
        // (NO_COLOR present → returns false).
        let no_color_set = std::env::var_os("NO_COLOR").is_some();
        assert_eq!(
            is_color_enabled(),
            !no_color_set,
            "is_color_enabled should be false when NO_COLOR is set, true otherwise"
        );
    }

    // --- darken proptest ---

    proptest! {
        #[test]
        fn darken_channels_never_increase(
            r in 0u8..=255,
            g in 0u8..=255,
            b in 0u8..=255,
            amt in 0.0f32..=1.0,
        ) {
            let result = darken(Color::Rgb(r, g, b), amt);
            if let Color::Rgb(dr, dg, db) = result {
                prop_assert!(dr <= r);
                prop_assert!(dg <= g);
                prop_assert!(db <= b);
            }
        }
    }
}
