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
//! - [`resolve_color`] — dispatch to the active [`ColorScheme`]
//! - [`resolve_color_mtime`] — blue→white→red mtime gradient
//! - [`resolve_color_depth`] — light→dark depth gradient

use std::ffi::OsStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ratatui::style::Color;

use crate::types::FileCategory;
use crate::ui::visualization::{ColorContext, ColorScheme};

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
    // Verified false positive: factor = (1.0 - amount) * channel, where
    // amount ∈ [0.0, 1.0] and channel ∈ [0, 255], so result ∈ [0.0, 255.0]
    // — no sign loss or truncation beyond the intended rounding.
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
///
/// The result is cached in a [`std::sync::OnceLock`] so the environment is
/// read only once per process, on the first call.
pub fn is_color_enabled() -> bool {
    static CACHED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| std::env::var_os("NO_COLOR").is_none())
}

// ---------------------------------------------------------------------------
// Color-scheme resolution
// ---------------------------------------------------------------------------

/// Constant colour channels for the mtime gradient endpoints and midpoint.
///
/// The gradient runs blue (old) → white (middle) → red (recent).
const MTIME_COLD: (u8, u8, u8) = (30, 100, 220); // blue
const MTIME_MID: (u8, u8, u8) = (255, 255, 255); // white
const MTIME_HOT: (u8, u8, u8) = (220, 40, 40); // red

/// Constant colour channels for the depth gradient endpoints.
///
/// Shallow (depth 0) is light; deepest level is dark.
const DEPTH_LIGHT: (u8, u8, u8) = (220, 220, 240);
const DEPTH_DARK: (u8, u8, u8) = (30, 30, 80);

/// Linearly interpolate between two `u8` colour channel values.
///
/// `t` is clamped to `[0.0, 1.0]`. The result is always in
/// `[min(a, b), max(a, b)]` and therefore a valid `u8`.
fn lerp_channel(a: u8, b: u8, t: f32) -> u8 {
    // mul_add(slope, t, a) = a + slope * t; result ∈ [min(a,b), max(a,b)] ⊆ [0.0, 255.0].
    let result = f32::mul_add(
        f32::from(b) - f32::from(a),
        t.clamp(0.0_f32, 1.0_f32),
        f32::from(a),
    );
    // Verified false positive: after rounding, result ≤ 255.0 and ≥ 0.0,
    // so it fits in u8. No sign loss; no truncation hazard.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    {
        result.round() as u8
    }
}

/// Map a normalised position `t ∈ [0, 1]` to the mtime colour gradient.
///
/// `t = 0` → cold blue; `t = 0.5` → white; `t = 1` → hot red.
fn mtime_gradient(t: f32) -> Color {
    let (r, g, b) = if t < 0.5_f32 {
        let s = t * 2.0_f32;
        (
            lerp_channel(MTIME_COLD.0, MTIME_MID.0, s),
            lerp_channel(MTIME_COLD.1, MTIME_MID.1, s),
            lerp_channel(MTIME_COLD.2, MTIME_MID.2, s),
        )
    } else {
        let s = (t - 0.5_f32) * 2.0_f32;
        (
            lerp_channel(MTIME_MID.0, MTIME_HOT.0, s),
            lerp_channel(MTIME_MID.1, MTIME_HOT.1, s),
            lerp_channel(MTIME_MID.2, MTIME_HOT.2, s),
        )
    };
    Color::Rgb(r, g, b)
}

/// Map a normalised position `t ∈ [0, 1]` to a grayscale mtime shade.
///
/// `t = 0` (old) → dark gray 40; `t = 1` (recent) → light gray 200.
fn mtime_grayscale(t: f32) -> Color {
    let v = lerp_channel(40, 200, t);
    Color::Rgb(v, v, v)
}

/// Map a normalised depth position `t ∈ [0, 1]` to the depth colour gradient.
///
/// `t = 0` (root) → light; `t = 1` (deepest) → dark.
fn depth_gradient(t: f32) -> Color {
    Color::Rgb(
        lerp_channel(DEPTH_LIGHT.0, DEPTH_DARK.0, t),
        lerp_channel(DEPTH_LIGHT.1, DEPTH_DARK.1, t),
        lerp_channel(DEPTH_LIGHT.2, DEPTH_DARK.2, t),
    )
}

/// Map a normalised depth position `t ∈ [0, 1]` to a grayscale depth shade.
///
/// `t = 0` (root) → light gray 220; `t = 1` (deepest) → dark gray 40.
fn depth_grayscale(t: f32) -> Color {
    let v = lerp_channel(220, 40, t);
    Color::Rgb(v, v, v)
}

/// Normalise `mtime` to a `[0.0, 1.0]` position within `ctx.time_range`.
///
/// Returns `0.5` (midpoint) when `time_range` is absent or when `min == max`.
fn mtime_norm(mtime: SystemTime, ctx: &ColorContext) -> f32 {
    let Some(ref tr) = ctx.time_range else {
        return 0.5_f32;
    };
    let Ok(range_dur) = tr.max.duration_since(tr.min) else {
        // min >= max: inverted or equal range
        return 0.5_f32;
    };
    if range_dur == Duration::ZERO {
        return 0.5_f32;
    }
    let offset_dur = mtime.duration_since(tr.min).unwrap_or(Duration::ZERO);
    // Clamp offset to [0, range] before dividing.
    let offset_clamped = offset_dur.min(range_dur);
    (offset_clamped.as_secs_f32() / range_dur.as_secs_f32()).clamp(0.0_f32, 1.0_f32)
}

/// Normalise `ctx.depth` to a `[0.0, 1.0]` position within `[0, ctx.max_depth]`.
///
/// Returns `0.0` (light end) when `max_depth == 0`.
fn depth_norm(ctx: &ColorContext) -> f32 {
    if ctx.max_depth == 0 {
        return 0.0_f32;
    }
    (f32::from(ctx.depth.min(ctx.max_depth)) / f32::from(ctx.max_depth)).clamp(0.0_f32, 1.0_f32)
}

/// Return the colour for a file based on its extension, using only
/// [`FileCategory`] → [`category_color`] / [`no_color_fallback`].
fn file_color(ext: Option<&OsStr>) -> Color {
    let cat = FileCategory::from_extension(ext);
    if is_color_enabled() {
        category_color(cat)
    } else {
        no_color_fallback(cat)
    }
}

/// Resolve a cell colour from the active [`ColorScheme`].
///
/// Dispatches to:
/// - `FileType` → [`category_color`] / [`no_color_fallback`] keyed on `ext`
/// - `Mtime` → [`resolve_color_mtime`] using `ctx.time_range` (defaults to the
///   cold end when no per-entry mtime is available in `ctx`)
/// - `Depth` → [`resolve_color_depth`] using `ctx.depth` and `ctx.max_depth`
///
/// All variants fall back to evenly-spaced grayscale when the `NO_COLOR`
/// environment variable is set.
pub fn resolve_color(ext: Option<&OsStr>, scheme: ColorScheme, ctx: &ColorContext) -> Color {
    match scheme {
        ColorScheme::FileType => file_color(ext),
        ColorScheme::Mtime => {
            // No per-entry mtime in ColorContext; use the range minimum (cold end)
            // as a neutral baseline. Callers with a concrete mtime should use
            // resolve_color_mtime directly.
            let mtime = ctx.time_range.as_ref().map_or(UNIX_EPOCH, |r| r.min);
            resolve_color_mtime(mtime, ctx)
        },
        ColorScheme::Depth => resolve_color_depth(ctx),
    }
}

/// Resolve a cell colour from modification time.
///
/// Linearly maps `mtime` within `ctx.time_range` to a blue→white→red gradient:
/// - old (`mtime ≈ min`) → cold blue `Rgb(30, 100, 220)`
/// - midpoint → white `Rgb(255, 255, 255)`
/// - recent (`mtime ≈ max`) → hot red `Rgb(220, 40, 40)`
///
/// Guards `min == max` by returning the midpoint white. Falls back to a
/// dark-to-light grayscale when `NO_COLOR` is set.
pub fn resolve_color_mtime(mtime: SystemTime, ctx: &ColorContext) -> Color {
    let t = mtime_norm(mtime, ctx);
    if is_color_enabled() {
        mtime_gradient(t)
    } else {
        mtime_grayscale(t)
    }
}

/// Resolve a cell colour from nesting depth.
///
/// Maps `ctx.depth / ctx.max_depth` to a light→dark gradient:
/// - root (depth 0) → `Rgb(220, 220, 240)`
/// - deepest level → `Rgb(30, 30, 80)`
///
/// Guards `max_depth == 0` by returning the light end. Falls back to a
/// light-to-dark grayscale when `NO_COLOR` is set.
pub fn resolve_color_depth(ctx: &ColorContext) -> Color {
    let t = depth_norm(ctx);
    if is_color_enabled() {
        depth_gradient(t)
    } else {
        depth_grayscale(t)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::SystemTime;

    use proptest::prelude::*;
    use ratatui::style::Color;

    use super::*;
    use crate::types::FileCategory;
    use crate::ui::visualization::{ColorContext, ColorScheme};

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

    // --- resolve_color ---

    #[test]
    fn resolve_color_filetype_matches_category_color() {
        let ctx = ColorContext {
            time_range: None,
            max_depth: 10,
            depth: 0,
        };
        let color = resolve_color(Some(OsStr::new("rs")), ColorScheme::FileType, &ctx);
        assert_eq!(color, category_color(FileCategory::Code));
    }

    // --- resolve_color_mtime ---

    #[test]
    fn resolve_color_mtime_min_is_cold() {
        use std::time::Duration;
        let min = SystemTime::UNIX_EPOCH;
        let max = SystemTime::UNIX_EPOCH + Duration::from_hours(8760);
        let ctx = ColorContext {
            time_range: Some(crate::ui::visualization::TimeRange { min, max }),
            max_depth: 0,
            depth: 0,
        };
        // Cold end of blue→white→red gradient: should have high blue, low red.
        let color = resolve_color_mtime(min, &ctx);
        if let Color::Rgb(r, _, b) = color {
            assert!(b > r);
        }
    }

    #[test]
    fn resolve_color_mtime_equal_range_no_panic() {
        let t = SystemTime::UNIX_EPOCH;
        let ctx = ColorContext {
            time_range: Some(crate::ui::visualization::TimeRange { min: t, max: t }),
            max_depth: 0,
            depth: 0,
        };
        let color = resolve_color_mtime(t, &ctx);
        assert!(matches!(color, Color::Rgb(..)));
    }

    // --- resolve_color_depth ---

    #[test]
    fn resolve_color_depth_zero_max_no_panic() {
        let ctx = ColorContext {
            time_range: None,
            max_depth: 0,
            depth: 0,
        };
        let color = resolve_color_depth(&ctx);
        assert!(matches!(color, Color::Rgb(..)));
    }

    #[test]
    fn resolve_color_depth_gradient_darker_with_depth() {
        let ctx_shallow = ColorContext {
            time_range: None,
            max_depth: 10,
            depth: 0,
        };
        let ctx_deep = ColorContext {
            time_range: None,
            max_depth: 10,
            depth: 10,
        };
        let shallow = resolve_color_depth(&ctx_shallow);
        let deep = resolve_color_depth(&ctx_deep);
        // Shallow should be lighter (higher channel values) than deep.
        if let (Color::Rgb(rs, gs, bs), Color::Rgb(rd, gd, bd)) = (shallow, deep) {
            assert!(
                u32::from(rs) + u32::from(gs) + u32::from(bs)
                    > u32::from(rd) + u32::from(gd) + u32::from(bd)
            );
        }
    }
}
