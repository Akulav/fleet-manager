//! Screen capture. This runs **in the helper process** (interactive session) —
//! a session-0 service cannot capture the user's desktop.
//!
//! Capture is only ever started on an explicit request from the master, the
//! tray icon is visible the whole time, and the master shows a banner while a
//! live session is active, so the user can tell when their screen is being
//! viewed.

use anyhow::{Context, Result};
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder};

/// Capture the primary monitor and return `(width, height, jpeg_bytes)`.
pub fn capture_primary(jpeg_quality: u8) -> Result<(u32, u32, Vec<u8>)> {
    let monitors = xcap::Monitor::all().context("enumerate monitors")?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary())
        .or_else(|| monitors.first())
        .context("no monitors found")?;

    let rgba = monitor.capture_image().context("capture image")?;
    let (w, h) = (rgba.width(), rgba.height());
    let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();

    let mut buf = Vec::new();
    let quality = jpeg_quality.clamp(10, 95);
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    encoder
        .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
        .context("jpeg encode")?;
    Ok((w, h, buf))
}
