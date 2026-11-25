//! Mipmaps gerados na CPU para as texturas dos mundos.
//!
//! O cliente web usa mipmaps e filtragem anisotrópica (até 8×) nas texturas de chão e casca; sem eles, o chão repetido
//! 68 vezes vira listras à distância. O glTF não leva mipmaps, então cada textura usada por um material toon ganha a
//! cadeia completa uma vez, com média feita em espaço linear (texturas sRGB) para não escurecer.

use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use std::collections::HashSet;

/// Texturas que precisam de mipmaps.
#[derive(Resource, Default, Debug)]
pub struct MipQueue {
    pending: HashSet<AssetId<Image>>,
    done: HashSet<AssetId<Image>>,
}

impl MipQueue {
    pub fn request(&mut self, id: AssetId<Image>) {
        if !self.done.contains(&id) {
            self.pending.insert(id);
        }
    }
}

fn srgb_to_lin(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lin_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Gera a cadeia de mipmaps de uma imagem RGBA8 (média 2×2). Devolve `None` para outros formatos.
pub fn build_mips(width: u32, height: u32, data: &[u8], srgb: bool) -> Option<(Vec<u8>, u32)> {
    if data.len() != (width * height * 4) as usize || width == 0 || height == 0 {
        return None;
    }
    let to_l = |c: u8| if srgb { srgb_to_lin(c) } else { f32::from(c) / 255.0 };
    let to_c = |v: f32| if srgb { lin_to_srgb(v) } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
    let mut out = data.to_vec();
    let (mut w, mut h) = (width, height);
    let mut prev = data.to_vec();
    let mut levels = 1;
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0f32; 4];
                let mut n = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(w - 1);
                    let sy = (y * 2 + dy).min(h - 1);
                    let i = ((sy * w + sx) * 4) as usize;
                    let a = f32::from(prev[i + 3]) / 255.0;
                    // média ponderada pelo alfa: bordas transparentes não escurecem a cor
                    for c in 0..3 {
                        acc[c] += to_l(prev[i + c]) * a;
                    }
                    acc[3] += a;
                    n += 1.0;
                }
                let o = ((y * nw + x) * 4) as usize;
                let a = acc[3] / n;
                for c in 0..3 {
                    next[o + c] = to_c(if acc[3] > 0.0 { acc[c] / acc[3] } else { 0.0 });
                }
                next[o + 3] = (a * 255.0 + 0.5) as u8;
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        w = nw;
        h = nh;
        levels += 1;
    }
    Some((out, levels))
}

pub fn generate_mips(mut q: ResMut<MipQueue>, mut images: ResMut<Assets<Image>>) {
    if q.pending.is_empty() {
        return;
    }
    let ids: Vec<_> = q.pending.iter().copied().collect();
    for id in ids {
        let Some(img) = images.get(id) else { continue };
        let fmt = img.texture_descriptor.format;
        let srgb = match fmt {
            TextureFormat::Rgba8UnormSrgb => true,
            TextureFormat::Rgba8Unorm => false,
            _ => {
                q.pending.remove(&id);
                q.done.insert(id);
                continue;
            }
        };
        if img.texture_descriptor.mip_level_count > 1 || img.texture_descriptor.size.depth_or_array_layers != 1 {
            q.pending.remove(&id);
            q.done.insert(id);
            continue;
        }
        let (w, h) = (img.texture_descriptor.size.width, img.texture_descriptor.size.height);
        let built = img.data.as_deref().and_then(|d| build_mips(w, h, d, srgb));
        q.pending.remove(&id);
        q.done.insert(id);
        let Some((data, levels)) = built else { continue };
        if let Some(mut img) = images.get_mut(id) {
            img.data = Some(data);
            img.texture_descriptor.mip_level_count = levels;
            let mut s = match &img.sampler {
                ImageSampler::Descriptor(d) => d.clone(),
                ImageSampler::Default => ImageSamplerDescriptor::default(),
            };
            s.mag_filter = ImageFilterMode::Linear;
            s.min_filter = ImageFilterMode::Linear;
            s.mipmap_filter = ImageFilterMode::Linear;
            s.anisotropy_clamp = 8;
            if s.address_mode_u == ImageAddressMode::ClampToBorder {
                s.address_mode_u = ImageAddressMode::ClampToEdge;
            }
            if s.address_mode_v == ImageAddressMode::ClampToBorder {
                s.address_mode_v = ImageAddressMode::ClampToEdge;
            }
            img.sampler = ImageSampler::Descriptor(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_sizes() {
        let data = vec![255u8; 8 * 4 * 4];
        let (out, levels) = build_mips(8, 4, &data, true).unwrap();
        assert_eq!(levels, 4); // 8×4, 4×2, 2×1, 1×1
        assert_eq!(out.len(), (32 + 8 + 2 + 1) * 4);
        assert!(out.iter().all(|&b| b == 255));
        assert!(build_mips(3, 3, &[0; 4], false).is_none());
    }

    #[test]
    fn transparent_texels_do_not_darken() {
        // um texel branco opaco e três transparentes pretos: a cor média continua branca, o alfa cai para 1/4
        let mut d = vec![0u8; 2 * 2 * 4];
        d[..4].copy_from_slice(&[255, 255, 255, 255]);
        let (out, _) = build_mips(2, 2, &d, true).unwrap();
        assert_eq!(&out[16..20], &[255, 255, 255, 64]);
    }
}
