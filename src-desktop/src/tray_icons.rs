//! Provider logos for the menu bar: the web UI PNGs (`public/providers`),
//! decoded once with rounded corners, shown in color.

#[cfg(any(not(target_os = "windows"), test))]
use std::{collections::HashMap, sync::OnceLock};

use tauri::image::Image;

#[cfg(any(not(target_os = "windows"), test))]
use crate::model::Provider;

#[cfg(any(not(target_os = "windows"), test))]
const CLAUDE: &[u8] = include_bytes!("../../public/providers/claude.png");
#[cfg(any(not(target_os = "windows"), test))]
const CODEX: &[u8] = include_bytes!("../../public/providers/codex.png");
#[cfg(any(not(target_os = "windows"), test))]
const GEMINI: &[u8] = include_bytes!("../../public/providers/gemini-cli.png");
#[cfg(any(not(target_os = "windows"), test))]
const ANTIGRAVITY: &[u8] = include_bytes!("../../public/providers/antigravity.png");
#[cfg(any(not(target_os = "windows"), test))]
const COPILOT: &[u8] = include_bytes!("../../public/providers/copilot.png");
#[cfg(any(not(target_os = "windows"), test))]
const CURSOR: &[u8] = include_bytes!("../../public/providers/cursor.png");
#[cfg(any(not(target_os = "windows"), test))]
const OPENCODE_GO: &[u8] = include_bytes!("../../public/providers/opencode.png");

/// Corner radius as a share of the icon size, the macOS app icon ratio.
#[cfg(any(not(target_os = "windows"), test))]
const CORNER_RATIO: f64 = 0.225;

/// Windows cannot show a text title beside a tray icon. Render the remaining
/// percentage into the icon itself; the tooltip supplies its account and unit.
#[cfg(any(target_os = "windows", test))]
pub fn quota_icon(remaining: u8) -> Image<'static> {
    const DIGITS: [[u8; 5]; 10] = [
        [7, 5, 5, 5, 7],
        [2, 6, 2, 2, 7],
        [7, 1, 7, 4, 7],
        [7, 1, 7, 1, 7],
        [5, 5, 7, 1, 1],
        [7, 4, 7, 1, 7],
        [7, 4, 7, 5, 7],
        [7, 1, 2, 2, 2],
        [7, 5, 7, 5, 7],
        [7, 5, 7, 1, 7],
    ];
    let remaining = remaining.min(100);
    let background: [u8; 4] = match remaining {
        0..=10 => [153, 27, 27, 255],
        11..=25 => [133, 77, 14, 255],
        _ => [30, 64, 175, 255],
    };
    let mut rgba = background.repeat(32 * 32);
    let text = remaining.to_string();
    let scale = if text.len() == 3 { 3 } else { 4 };
    let width = text.len() * 3 * scale + (text.len() - 1) * 2;
    let left = (32 - width) / 2;
    let top = (26 - 5 * scale) / 2;
    for (position, digit) in text.bytes().enumerate() {
        for (row, bits) in DIGITS[(digit - b'0') as usize].iter().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let x = left + position * (3 * scale + 2) + column * scale + dx;
                        let y = top + row * scale + dy;
                        rgba[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&[255; 4]);
                    }
                }
            }
        }
    }
    let fill = (usize::from(remaining) * 24 + 50) / 100;
    for y in 28..30 {
        for x in 4..28 {
            let color = if x - 4 < fill {
                [255; 4]
            } else {
                [15, 23, 42, 255]
            };
            rgba[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].copy_from_slice(&color);
        }
    }
    Image::new_owned(rgba, 32, 32)
}

#[cfg(any(not(target_os = "windows"), test))]
fn cache() -> &'static HashMap<&'static str, Image<'static>> {
    static ICONS: OnceLock<HashMap<&'static str, Image<'static>>> = OnceLock::new();
    ICONS.get_or_init(|| {
        [
            ("claude", CLAUDE),
            ("codex", CODEX),
            ("gemini-cli", GEMINI),
            ("antigravity", ANTIGRAVITY),
            ("copilot", COPILOT),
            ("cursor", CURSOR),
            ("opencode-go", OPENCODE_GO),
        ]
        .into_iter()
        .filter_map(|(id, bytes)| Some((id, rounded(Image::from_bytes(bytes).ok()?))))
        .collect()
    })
}

/// A copy of the provider logo, or None when the PNG failed to decode.
#[cfg(any(not(target_os = "windows"), test))]
pub fn provider_icon(provider: &Provider) -> Option<Image<'static>> {
    let icon = cache().get(provider.as_str())?;
    Some(Image::new_owned(
        icon.rgba().to_vec(),
        icon.width(),
        icon.height(),
    ))
}

/// Clears the alpha outside a rounded rectangle, with a soft one-pixel edge.
#[cfg(any(not(target_os = "windows"), test))]
fn rounded(image: Image<'_>) -> Image<'static> {
    let (width, height) = (image.width(), image.height());
    let radius = f64::from(width.min(height)) * CORNER_RATIO;
    let mut rgba = image.rgba().to_vec();
    for y in 0..height {
        for x in 0..width {
            let coverage = corner_coverage(x, y, width, height, radius);
            if coverage < 1.0 {
                let alpha = &mut rgba[((y * width + x) * 4 + 3) as usize];
                *alpha = (f64::from(*alpha) * coverage).round() as u8;
            }
        }
    }
    Image::new_owned(rgba, width, height)
}

/// 1.0 inside the rounded rectangle, 0.0 outside, in between on the edge.
#[cfg(any(not(target_os = "windows"), test))]
fn corner_coverage(x: u32, y: u32, width: u32, height: u32, radius: f64) -> f64 {
    let px = f64::from(x) + 0.5;
    let py = f64::from(y) + 0.5;
    let cx = px.clamp(radius, f64::from(width) - radius);
    let cy = py.clamp(radius, f64::from(height) - radius);
    let distance = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
    (radius - distance + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_quota_icon_handles_every_percentage() {
        for remaining in 0..=100 {
            let icon = quota_icon(remaining);
            assert_eq!((icon.width(), icon.height()), (32, 32));
            assert_eq!(icon.rgba().len(), 32 * 32 * 4);
            assert!(icon.rgba().chunks_exact(4).all(|pixel| pixel[3] == 255));
            assert!(icon.rgba().chunks_exact(4).any(|pixel| pixel == [255; 4]));
        }
        assert_eq!(quota_icon(255).rgba(), quota_icon(100).rgba());
        assert_ne!(quota_icon(0).rgba(), quota_icon(100).rgba());
    }

    #[test]
    fn decodes_every_provider_logo() {
        for provider in [
            Provider::Claude,
            Provider::Codex,
            Provider::Gemini,
            Provider::Antigravity,
            Provider::Copilot,
            Provider::Cursor,
            Provider::OpenCodeGo,
        ] {
            let icon = provider_icon(&provider).expect("logo decodes");
            assert_eq!(icon.width(), 128);
            // The corner pixel is transparent, the center keeps its alpha.
            assert_eq!(icon.rgba()[3], 0);
            let center = ((64 * 128 + 64) * 4 + 3) as usize;
            assert_eq!(icon.rgba()[center], 255);
        }
    }
}
