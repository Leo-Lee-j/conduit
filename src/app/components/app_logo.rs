//! Resample the supplied logo once per device-pixel size, before GPU painting.

use super::super::*;
use std::{
    collections::VecDeque,
    sync::{LazyLock, Mutex},
};

// Filtering straight-alpha pixels can pull colors from transparent corners
// into the outline. Keep the decoded original premultiplied while resizing.
static SOURCE: LazyLock<image::RgbaImage> = LazyLock::new(|| {
    let mut image = image::load_from_memory_with_format(
        include_bytes!("../../../assets/app-icons/source.png"),
        image::ImageFormat::Png,
    )
    .expect("valid application logo")
    .into_rgba8();
    for pixel in image.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    image
});

// Display sizes are small, but moving between DPI settings can introduce new
// entries. Keep a bounded LRU rather than retaining every size ever requested.
const CACHE_LIMIT: usize = 16;
type LogoCache = VecDeque<(u32, Arc<RenderImage>)>;
static IMAGES: LazyLock<Mutex<LogoCache>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

fn rasterize(pixels: u32) -> Arc<RenderImage> {
    let mut image = image::imageops::resize(
        &*SOURCE,
        pixels,
        pixels,
        image::imageops::FilterType::Lanczos3,
    );
    for pixel in image.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = (u32::from(*channel) * 255 + alpha / 2)
                .checked_div(alpha)
                .unwrap_or(0)
                .min(255) as u8;
        }
        // RenderImage consumes straight-alpha BGRA, not the PNG's RGBA.
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
}

fn rendered_logo(pixels: u32) -> Arc<RenderImage> {
    let mut cache = IMAGES.lock().expect("application logo cache");
    let image = if let Some(index) = cache.iter().position(|(size, _)| *size == pixels) {
        cache.remove(index).expect("cached logo").1
    } else {
        rasterize(pixels)
    };
    cache.push_back((pixels, image.clone()));
    while cache.len() > CACHE_LIMIT {
        cache.pop_front();
    }
    image
}

pub(in crate::app) fn app_icon(logical_size: f32) -> Div {
    div().size(px(logical_size)).flex_shrink_0().child(
        canvas(
            |bounds, window, _| {
                let scale = window.scale_factor();
                let pixels = (f32::from(bounds.size.width.min(bounds.size.height)) * scale)
                    .round()
                    .max(1.) as u32;
                let extent = px(pixels as f32 / scale);
                let center = bounds.center();
                let origin = point(
                    px(((f32::from(center.x) * scale - pixels as f32 / 2.).round()) / scale),
                    px(((f32::from(center.y) * scale - pixels as f32 / 2.).round()) / scale),
                );
                (
                    rendered_logo(pixels),
                    Bounds::new(origin, size(extent, extent)),
                )
            },
            |_, (image, bounds), window, _| {
                let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
            },
        )
        .size_full(),
    )
}
