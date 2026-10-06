//! Text for the 3D cube (face captions, axis letters).
//!
//! The text is rasterised once with `swash` from the same [`Font`] asset
//! `bevy_text` loads, into a small alpha texture shown on an unlit, alpha-blended
//! quad. Unlike UI text it lives in the cube's 3D scene, so it turns with the
//! face it's printed on, and unlike extruded glyph meshes it needs no extra font
//! crate. The quad is sized in *ems* (1 em = 1 world unit), centered on the
//! text's ink, so a [`Transform`] scale sets the font size.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;
use swash::FontRef;

/// Text to show on a quad on this entity. Add a [`Transform`] (scale = font
/// size in world units per em) and render layers; the mesh, material and
/// texture are filled in once the font has loaded.
#[derive(Component, Debug, Clone)]
pub(crate) struct Caption {
    pub text: String,
    pub font: Handle<Font>,
    pub color: Color,
    /// Size (px per em) the glyphs are rasterised at. Pick about the on-screen
    /// size, a little over: much larger just aliases when shrunk.
    pub raster_px: f32,
    /// Extra stroke weight, in ems (0.04 ≈ a touch bolder, 0.08 heavy). The
    /// outline is grown while rasterising, so it stays crisp; a thin face
    /// shrunk to a few pixels otherwise reads weak.
    pub bold: f32,
}

impl Caption {
    pub fn new(
        text: impl Into<String>,
        font: Handle<Font>,
        color: Color,
        raster_px: f32,
        bold: f32,
    ) -> Self {
        Self {
            text: text.into(),
            font,
            color,
            raster_px,
            bold,
        }
    }
}

/// Marks a [`Caption`] whose quad has been built.
#[derive(Component)]
struct CaptionBuilt;

pub(crate) struct CaptionPlugin;

impl Plugin for CaptionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, build_captions);
    }
}

fn build_captions(
    mut commands: Commands,
    fonts: Res<Assets<Font>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    pending: Query<(Entity, &Caption), Without<CaptionBuilt>>,
) {
    for (entity, caption) in &pending {
        // The font loads asynchronously; try again next frame.
        let Some(font) = fonts.get(&caption.font) else {
            continue;
        };
        let mut entity = commands.entity(entity);
        entity.insert(CaptionBuilt);
        let Some(raster) = rasterise(
            font.data.as_ref(),
            &caption.text,
            caption.raster_px,
            caption.bold,
        ) else {
            continue;
        };
        let (w, h) = (raster.width as f32, raster.height as f32);
        // Plain (non-sRGB) RGBA: the glyphs are white, only alpha matters, and
        // it is the format every GPU, mobile WebGL included, samples reliably.
        let (data, mips) = with_mips(raster.width, raster.height, &raster.rgba);
        let mut image = Image::new(
            Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            raster.rgba,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.data = Some(data);
        image.texture_descriptor.mip_level_count = mips;
        // Trilinear, but at most one level down: deeper levels wash the thin
        // strokes out.
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            lod_max_clamp: 1.0,
            ..ImageSamplerDescriptor::linear()
        });
        entity.insert((
            Mesh3d(meshes.add(Rectangle::new(w / caption.raster_px, h / caption.raster_px))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: caption.color,
                base_color_texture: Some(images.add(image)),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            })),
        ));
    }
}

/// `rgba` followed by every smaller mip level (2×2 box filter), and the level
/// count. Without mips, text shrunk on a high-DPI or small screen shimmers, and
/// a trilinear sampler on a one-level texture upsets some mobile GL drivers.
fn with_mips(width: u32, height: u32, rgba: &[u8]) -> (Vec<u8>, u32) {
    let mut data = rgba.to_vec();
    let (mut w, mut h) = (width as usize, height as usize);
    let mut level = 1;
    let mut prev = rgba.to_vec();
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let at = |sx: usize, sy: usize| {
                        prev[(sy.min(h - 1) * w + sx.min(w - 1)) * 4 + c] as u32
                    };
                    let sum = at(2 * x, 2 * y)
                        + at(2 * x + 1, 2 * y)
                        + at(2 * x, 2 * y + 1)
                        + at(2 * x + 1, 2 * y + 1);
                    next[(y * nw + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        prev = next;
        (w, h) = (nw, nh);
        level += 1;
    }
    (data, level)
}

/// White RGBA pixels whose alpha is the text's coverage, tightly cropped to the
/// ink (plus a pixel of padding so filtering doesn't clip the edge).
struct Raster {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn rasterise(font_data: &[u8], text: &str, px: f32, bold: f32) -> Option<Raster> {
    let font = FontRef::from_index(font_data, 0)?;
    let charmap = font.charmap();
    let metrics = font.glyph_metrics(&[]).scale(px);
    let mut context = ScaleContext::new();
    let mut scaler = context.builder(font).size(px).hint(false).build();

    // (pen x, glyph image) for every glyph that has ink.
    let embolden = bold * px;
    let mut glyphs = Vec::new();
    let mut pen = 0.0f32;
    for ch in text.chars() {
        let id = charmap.map(ch);
        if let Some(image) = Render::new(&[Source::Outline])
            .format(Format::Alpha)
            .embolden(embolden)
            .render(&mut scaler, id)
        {
            glyphs.push((pen, image));
        }
        // Bolder strokes eat into the gap to the next glyph; widen the advance.
        pen += metrics.advance_width(id) + embolden;
    }
    if glyphs.is_empty() {
        return None;
    }

    // Ink bounds in canvas coordinates (x right, y down from the baseline).
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (pen, g) in &glyphs {
        let p = &g.placement;
        let x = pen + p.left as f32;
        let y = -(p.top as f32);
        min_x = min_x.min(x);
        max_x = max_x.max(x + p.width as f32);
        min_y = min_y.min(y);
        max_y = max_y.max(y + p.height as f32);
    }
    const PAD: i32 = 1;
    let origin_x = min_x.floor() as i32 - PAD;
    let origin_y = min_y.floor() as i32 - PAD;
    let width = (max_x.ceil() as i32 - origin_x + PAD).max(1) as u32;
    let height = (max_y.ceil() as i32 - origin_y + PAD).max(1) as u32;

    let mut rgba = vec![255u8; (width * height * 4) as usize];
    for texel in rgba.chunks_exact_mut(4) {
        texel[3] = 0;
    }
    for (pen, g) in &glyphs {
        let p = &g.placement;
        let left = (pen + p.left as f32).round() as i32 - origin_x;
        let top = -p.top - origin_y;
        for row in 0..p.height as i32 {
            for col in 0..p.width as i32 {
                let (x, y) = (left + col, top + row);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let coverage = g.data[(row as u32 * p.width + col as u32) as usize];
                let alpha = &mut rgba[((y as u32 * width + x as u32) * 4 + 3) as usize];
                // Neighbouring glyphs can overlap by a pixel: keep the denser.
                *alpha = (*alpha).max(coverage);
            }
        }
    }
    Some(Raster {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: &[u8] = bevy::text::DEFAULT_FONT_DATA;

    #[test]
    fn rasterises_ink_tightly() {
        let r = rasterise(FONT, "FRONT", 48.0, 0.0).expect("glyphs");
        assert_eq!(r.rgba.len(), (r.width * r.height * 4) as usize);
        // Cropped to the ink: about 5 caps wide and one cap tall.
        assert!(r.width > 100 && r.width < 220, "{}", r.width);
        assert!(r.height > 25 && r.height < 50, "{}", r.height);
        // Real coverage, and the padding column is empty.
        let opaque = r.rgba.chunks_exact(4).filter(|p| p[3] > 128).count();
        assert!(opaque > 200, "{opaque}");
        for y in 0..r.height {
            assert_eq!(r.rgba[((y * r.width) * 4 + 3) as usize], 0);
        }
    }

    #[test]
    fn longer_text_is_wider_and_empty_is_none() {
        let a = rasterise(FONT, "TOP", 48.0, 0.0).unwrap();
        let b = rasterise(FONT, "BOTTOM", 48.0, 0.0).unwrap();
        assert!(b.width > a.width);
        assert!(rasterise(FONT, "", 48.0, 0.0).is_none());
    }

    #[test]
    fn mip_chain_is_complete() {
        let (w, h) = (37u32, 13u32);
        let base: Vec<u8> = (0..w * h * 4).map(|i| (i % 251) as u8).collect();
        let (data, levels) = with_mips(w, h, &base);
        assert_eq!(levels, 6); // 37 -> 18 -> 9 -> 4 -> 2 -> 1
        let (mut mw, mut mh, mut total) = (w as usize, h as usize, 0);
        for _ in 0..levels {
            total += mw * mh * 4;
            (mw, mh) = ((mw / 2).max(1), (mh / 2).max(1));
        }
        assert_eq!(data.len(), total);
        assert_eq!(&data[..base.len()], &base[..]);
        // A solid texture stays solid down the chain.
        let (solid, n) = with_mips(8, 8, &[255; 8 * 8 * 4]);
        assert_eq!(n, 4);
        assert!(solid.iter().all(|&b| b == 255));
    }

    #[test]
    fn bold_adds_ink() {
        let ink = |bold| {
            rasterise(FONT, "FRONT", 48.0, bold)
                .unwrap()
                .rgba
                .chunks_exact(4)
                .map(|p| p[3] as u32)
                .sum::<u32>()
        };
        let (regular, bolder) = (ink(0.0), ink(0.05));
        assert!(bolder as f32 > regular as f32 * 1.15, "{regular} -> {bolder}");
    }
}
