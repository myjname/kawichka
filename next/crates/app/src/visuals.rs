//! Bounded decoding of data-only visual presets, including the existing atlas.
use image::{AnimationDecoder, GenericImageView};
use std::path::Path;
pub struct Frames {
    pub frames: Vec<(image::RgbaImage, u32)>,
    pub atlas: bool,
}
fn small(image: image::RgbaImage) -> image::RgbaImage {
    if image.width() > 520 || image.height() > 520 {
        image::imageops::thumbnail(&image, 520, 520)
    } else {
        image
    }
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
        return Ok(Frames {
            frames,
            atlas: true,
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
            frames.push((small(img), (n / d.max(1)).clamp(20, 10000)));
        }
        if frames.is_empty() {
            return Err("Empty animation".into());
        }
        return Ok(Frames {
            frames,
            atlas: false,
        });
    }
    let img = image::open(path).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Frames {
        frames: vec![(small(img), 1000)],
        atlas: false,
    })
}
impl Frames {
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
        let margin = 8. * scale;
        let available_w = (width as f32 - 2. * margin).max(0.);
        let available_h = (height as f32 - 2. * margin).max(0.);
        let image = &self.frames[index].0;
        let ratio = (available_w / image.width() as f32).min(available_h / image.height() as f32);
        let w = (image.width() as f32 * ratio).round() as u32;
        let h = (image.height() as f32 * ratio).round() as u32;
        if w == 0 || h == 0 {
            return vec![];
        }
        let img = image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle);
        let left = ((width - w) / 2) as i32;
        let top = ((height - h) / 2) as i32;
        // Rasterize controls into the same row runs: sorted, disjoint spans
        // let Windows build the entire native region in one bounded call.
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            [
                ((8. + x) * scale).round() as i32,
                ((8. + y) * scale).round() as i32,
                (((8. + x + w) * scale).round() as i32).min(width as i32),
                (((8. + y + h) * scale).round() as i32).min(height as i32),
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
    pub fn image(&self, index: usize) -> slint::Image {
        let img = &self.frames[index].0;
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            img.as_raw(),
            img.width(),
            img.height(),
        );
        slint::Image::from_rgba8(buffer)
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
}
