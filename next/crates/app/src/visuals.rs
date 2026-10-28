//! Bounded decoding of data-only visual presets, including the existing atlas.
use image::{AnimationDecoder, GenericImageView};
use std::path::Path;
pub struct Frames {
    pub frames: Vec<(image::RgbaImage, u32)>,
    pub atlas: bool,
    /// Pixel sprites use a physical-pixel grid; regular artwork stays smooth.
    pub pixelated: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}
fn small(image: image::RgbaImage, pixelated: bool) -> image::RgbaImage {
    if image.width() > 520 || image.height() > 520 {
        if pixelated {
            let ratio = (520. / image.width() as f64).min(520. / image.height() as f64);
            image::imageops::resize(
                &image,
                (image.width() as f64 * ratio).round().max(1.) as u32,
                (image.height() as f64 * ratio).round().max(1.) as u32,
                image::imageops::FilterType::Nearest,
            )
        } else {
            image::imageops::thumbnail(&image, 520, 520)
        }
    } else {
        image
    }
}
fn is_pixel_art(frames: &[(image::RgbaImage, u32)]) -> bool {
    let mut colors = std::collections::HashSet::with_capacity(65);
    for (image, _) in frames {
        if image.width() > 256 || image.height() > 256 {
            return false;
        }
        for pixel in image.pixels() {
            if pixel.0[3] != 0 && pixel.0[3] != 255 {
                return false;
            }
            if pixel.0[3] != 0 {
                colors.insert(pixel.0);
                if colors.len() > 64 {
                    return false;
                }
            }
        }
    }
    !colors.is_empty()
}
pub fn validate(path: &Path, layout: &str) -> Result<(), String> {
    load(path, layout).map(|_| ())
}
pub fn load(path: &Path, layout: &str) -> Result<Frames, String> {
    kavychka_storage::preview::no_link_chain(path).map_err(|e| e.to_string())?;
    if path.metadata().map_err(|e| e.to_string())?.len() > 200 * 1024 * 1024 {
        return Err("Image exceeds 200 MiB".into());
    }
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > 40_000_000 {
        return Err("Image exceeds 40 million pixels".into());
    }
    if layout == "codex" && (w, h) != (1536, 1872) {
        return Err("Codex atlas must be 1536 x 1872".into());
    }
    let atlas = layout == "codex" || layout == "auto" && (w, h) == (1536, 1872);
    if atlas {
        let img = image::open(path).map_err(|e| e.to_string())?;
        let mut frames = vec![];
        for y in 0..9 {
            for x in 0..8 {
                frames.push((img.view(x * 192, y * 208, 192, 208).to_image(), 140));
            }
        }
        let pixelated = is_pixel_art(&frames);
        return Ok(Frames {
            frames,
            atlas: true,
            pixelated,
        });
    }
    if path
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("gif"))
    {
        let decoder = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(
            std::fs::File::open(path).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;
        let mut frames = vec![];
        let mut pixels = 0u64;
        for f in decoder.into_frames().take(301) {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            let img = f.into_buffer();
            pixels += u64::from(img.width()) * u64::from(img.height());
            if frames.len() >= 300 || pixels > 40_000_000 {
                return Err("Animation exceeds 300 frames / 40 million decoded pixels".into());
            }
            frames.push((
                small(img, layout == "pixel"),
                (n / d.max(1)).clamp(20, 10000),
            ));
        }
        if frames.is_empty() {
            return Err("Empty animation".into());
        }
        let pixelated = layout == "pixel" || layout != "smooth" && is_pixel_art(&frames);
        return Ok(Frames {
            frames,
            atlas: false,
            pixelated,
        });
    }
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    let frames = vec![(small(img, layout == "pixel"), 1000)];
    let pixelated = layout == "pixel" || layout != "smooth" && is_pixel_art(&frames);
    Ok(Frames {
        frames,
        atlas: false,
        pixelated,
    })
}
impl Frames {
    fn raster(&self, index: usize, placement: Placement) -> image::RgbaImage {
        image::imageops::resize(
            &self.frames[index].0,
            placement.width,
            placement.height,
            if self.pixelated {
                image::imageops::FilterType::Nearest
            } else {
                image::imageops::FilterType::Triangle
            },
        )
    }
    /// Pre-scale pixel sprites into physical pixels, so Slint displays a 1:1
    /// texture instead of shifting alpha edges with a second nearest sampler.
    /// Regular artwork retains Slint's original premultiplied smooth filter.
    pub fn image_at(&self, index: usize, placement: Placement) -> slint::Image {
        if placement.width == 0 || placement.height == 0 {
            return slint::Image::default();
        }
        let scaled;
        let img = if self.pixelated {
            scaled = self.raster(index, placement);
            &scaled
        } else {
            &self.frames[index].0
        };
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            img.as_raw(),
            img.width(),
            img.height(),
        );
        slint::Image::from_rgba8(buffer)
    }
    /// The visible rectangle and the native click mask share this geometry.
    /// Integer upscaling avoids alternating pixel widths at fractional DPI.
    pub fn placement(&self, index: usize, width: u32, height: u32, scale: f32) -> Placement {
        let empty = Placement {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        };
        let Some((image, _)) = self.frames.get(index) else {
            return empty;
        };
        if width > 4096 || height > 4096 || !scale.is_finite() || scale <= 0. {
            return empty;
        }
        let margin = (8. * scale).round() as u32;
        let available_w = width.saturating_sub(margin.saturating_mul(2));
        let available_h = height.saturating_sub(margin.saturating_mul(2));
        let mut ratio = (available_w as f64 / image.width() as f64)
            .min(available_h as f64 / image.height() as f64);
        if self.pixelated && ratio >= 1. {
            ratio = ratio.floor();
        }
        let w = (image.width() as f64 * ratio).round() as u32;
        let h = (image.height() as f64 * ratio).round() as u32;
        Placement {
            left: width.saturating_sub(w) / 2,
            top: height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
    }
    /// Exact contain geometry used by PetWindow. Alpha < 8 is click-through,
    /// including holes; never union frames (that would fill animated holes).
    pub fn hit_spans(
        &self,
        index: usize,
        width: u32,
        height: u32,
        paused: bool,
        pending: bool,
        scale: f32,
    ) -> Vec<[i32; 4]> {
        if width > 4096 || height > 4096 {
            return vec![];
        }
        let placement = self.placement(index, width, height, scale);
        let w = placement.width;
        let h = placement.height;
        if w == 0 || h == 0 {
            return vec![];
        }
        let img = self.raster(index, placement);
        let left = placement.left as i32;
        let top = placement.top as i32;
        // Rasterize controls into the same row runs: sorted, disjoint spans
        // let Windows build the entire native region in one bounded call.
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            let available_w = (width as f32 - 16. * scale).max(0.);
            let available_h = (height as f32 - 16. * scale).max(0.);
            [
                (8. * scale + x * available_w / 100.).floor() as i32,
                (8. * scale + y * available_h / 100.).floor() as i32,
                (8. * scale + (x + w) * available_w / 100.).ceil() as i32,
                (8. * scale + (y + h) * available_h / 100.).ceil() as i32,
            ]
        };
        let mut controls = vec![];
        if paused {
            for i in 0..4 {
                controls.push(rect(14. + i as f32 * 23., 10., 3., 66.));
            }
            controls.push(rect(3., 77., 94., 19.));
        } else if pending {
            controls.push(rect(68., 73., 25., 22.));
        }
        let mut result = vec![];
        for y in 0..height as i32 {
            let mut start = None;
            for x in 0..=width as i32 {
                let in_image = x >= left && x < left + w as i32 && y >= top && y < top + h as i32;
                let opaque = x < width as i32
                    && ((in_image && img.get_pixel((x - left) as u32, (y - top) as u32).0[3] >= 8)
                        || controls
                            .iter()
                            .any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3]));
                if opaque && start.is_none() {
                    start = Some(x);
                }
                if !opaque && let Some(from) = start.take() {
                    result.push([from, y, x, y + 1]);
                }
            }
        }
        result
    }
    pub fn index(
        &self,
        elapsed: u64,
        paused: bool,
        speaking: bool,
        teasing: bool,
        happy: bool,
    ) -> usize {
        if self.atlas {
            let row = if paused {
                0
            } else if happy {
                4
            } else if speaking {
                3
            } else if teasing {
                8
            } else {
                0
            };
            let col = if paused {
                0
            } else if row == 0 {
                let mut t = elapsed % 6600;
                let mut col = 0;
                for (i, ms) in [1680, 660, 660, 840, 840, 1920].iter().enumerate() {
                    if t < *ms {
                        col = i;
                        break;
                    }
                    t -= *ms
                }
                col
            } else {
                (elapsed / 140
                    % if row == 4 {
                        5
                    } else if row == 3 {
                        4
                    } else {
                        6
                    }) as usize
            };
            row * 8 + col
        } else if paused {
            0
        } else {
            let total: u64 = self.frames.iter().map(|(_, ms)| u64::from(*ms)).sum();
            let mut t = elapsed % total;
            let mut index = 0;
            for (i, (_, ms)) in self.frames.iter().enumerate() {
                if t < u64::from(*ms) {
                    index = i;
                    break;
                }
                t -= u64::from(*ms)
            }
            index
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contains(spans: &[[i32; 4]], x: i32, y: i32) -> bool {
        spans
            .iter()
            .any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
    }
    #[test]
    fn alpha_mask_preserves_holes_and_contains_geometry_at_dpi() {
        let img = image::RgbaImage::from_fn(100, 100, |x, y| {
            image::Rgba([
                200,
                100,
                10,
                if (30..70).contains(&x) && (30..70).contains(&y) {
                    0
                } else {
                    255
                },
            ])
        });
        let f = Frames {
            frames: vec![(img, 100)],
            atlas: false,
            pixelated: false,
        };
        for scale in [1., 1.5, 2.] {
            let size = (116. * scale) as u32;
            let spans = f.hit_spans(0, size, size, false, false, scale);
            assert!(!contains(
                &spans,
                (58. * scale) as i32,
                (58. * scale) as i32
            ));
            assert!(contains(&spans, (20. * scale) as i32, (20. * scale) as i32));
            assert!(!contains(&spans, 0, 0));
        }
    }
    #[test]
    fn transparent_frames_letterboxing_and_badges() {
        let f = Frames {
            frames: vec![
                (image::RgbaImage::new(200, 100), 100),
                (
                    image::RgbaImage::from_pixel(200, 100, image::Rgba([0, 0, 0, 255])),
                    100,
                ),
            ],
            atlas: false,
            pixelated: false,
        };
        assert!(f.hit_spans(0, 116, 116, false, false, 1.).is_empty());
        assert!(!contains(
            &f.hit_spans(1, 116, 116, false, false, 1.),
            58,
            20
        ));
        assert!(contains(
            &f.hit_spans(1, 116, 116, false, false, 1.),
            58,
            58
        ));
        assert!(contains(&f.hit_spans(0, 116, 116, false, true, 1.), 80, 85));
        assert!(contains(&f.hit_spans(0, 116, 116, true, false, 1.), 23, 30));
        assert!(f.hit_spans(0, 5000, 116, false, false, 1.).is_empty());
    }
    #[test]
    fn pixel_geometry_preserves_integer_blocks_and_mask_for_all_sizes_and_dpi() {
        let image = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba(
                if x < 4
                    || y < 4
                    || x > 59
                    || y > 59
                    || (24..40).contains(&x) && (24..40).contains(&y)
                {
                    [0, 0, 0, 0]
                } else if x % 2 == 0 {
                    [230, 30, 80, 255]
                } else {
                    [100, 170, 240, 255]
                },
            )
        });
        let f = Frames {
            frames: vec![(image, 100)],
            atlas: false,
            pixelated: true,
        };
        for scale in [1., 1.25, 1.5, 1.75, 2.] {
            for logical_size in 80..=260 {
                let physical = (logical_size as f32 * scale).round() as u32;
                let p = f.placement(0, physical, physical, scale);
                assert_eq!(p.width, p.height);
                assert_eq!(p.width % 64, 0);
                assert_eq!(p.height % 64, 0);
                assert!(p.width >= 64);
                assert!(p.left + p.width <= physical);
                assert!(p.top + p.height <= physical);
                let spans = f.hit_spans(0, physical, physical, false, false, scale);
                let factor = p.width / 64;
                for (x, y) in [(0, 0), (12, 12), (32, 32), (63, 63), (50, 50)] {
                    let expected = f.frames[0].0.get_pixel(x, y).0[3] >= 8;
                    for dy in 0..factor {
                        for dx in 0..factor {
                            assert_eq!(
                                contains(
                                    &spans,
                                    (p.left + x * factor + dx) as i32,
                                    (p.top + y * factor + dy) as i32
                                ),
                                expected
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn pixel_detection_and_explicit_overrides_preserve_regular_artwork() {
        let sprite = image::RgbaImage::from_fn(64, 64, |x, y| {
            image::Rgba(if x == y {
                [240, 50, 70, 255]
            } else {
                [0, 0, 0, 0]
            })
        });
        assert!(is_pixel_art(&[(sprite.clone(), 100)]));
        let art = image::RgbaImage::from_fn(64, 64, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        assert!(!is_pixel_art(&[(art, 100)]));
        assert!(!is_pixel_art(&[(
            image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 30, 40, 100])),
            100
        )]));
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("sprite.png");
        sprite.save(&file).unwrap();
        assert!(load(&file, "auto").unwrap().pixelated);
        assert!(load(&file, "image").unwrap().pixelated);
        assert!(load(&file, "pixel").unwrap().pixelated);
        assert!(!load(&file, "smooth").unwrap().pixelated);
        assert_eq!(load(&file, "image").unwrap().frames[0].0, sprite);
        let f = load(&file, "smooth").unwrap();
        let p = f.placement(0, 130, 110, 1.25);
        assert_eq!(p.width, p.height);
        assert_eq!(p.height, 90);
        assert!(f.placement(0, 100, 100, f32::NAN).width == 0);
    }
    #[test]
    fn existing_codex_atlas_still_decodes_its_animation_rows() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("atlas.png");
        image::RgbaImage::from_fn(1536, 1872, |x, y| {
            image::Rgba([(x / 192) as u8, (y / 208) as u8, 10, 255])
        })
        .save(&path)
        .unwrap();
        for layout in ["auto", "codex"] {
            let f = load(&path, layout).unwrap();
            assert!(f.atlas && !f.pixelated);
            assert_eq!(f.frames.len(), 72);
            assert_eq!(f.frames[71].0.dimensions(), (192, 208));
            assert_eq!(f.frames[71].0.get_pixel(0, 0).0, [7, 8, 10, 255]);
            assert_eq!(f.index(0, true, false, false, false), 0);
            assert_eq!(f.index(420, false, true, false, false), 27);
            assert_eq!(f.index(560, false, false, false, true), 36);
            assert_eq!(f.index(700, false, false, true, false), 69);
        }
        image::RgbaImage::from_fn(1536, 1872, |x, _| {
            image::Rgba(if x % 2 == 0 {
                [20, 50, 80, 255]
            } else {
                [40, 60, 100, 255]
            })
        })
        .save(&path)
        .unwrap();
        assert!(load(&path, "codex").unwrap().pixelated);
    }
}
