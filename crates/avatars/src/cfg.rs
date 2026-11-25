//! Visual do avatar (`cfg`) e paletas do catálogo original (`data/catalogs/catalog.json`).

use serde::Deserialize;
use std::collections::BTreeMap;

/// Paletas e faixas do catálogo que o avatar usa.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub struct Palettes {
    pub pal: Vec<String>,
    pub skin: Vec<String>,
    pub eye: Vec<String>,
    pub hairc: Vec<String>,
    /// Versões brilhantes das paletas (`PAL`, `SKIN`, `EYE`, `HAIRC`), no mesmo índice.
    pub gloss: BTreeMap<String, Vec<String>>,
    /// Quantidade de opções de cada campo do `cfg` (validação, como `CR` no original).
    pub cr: BTreeMap<String, u32>,
    /// Opções retiradas: valor antigo → substituto (`CFG_GONE`).
    #[serde(default)]
    pub cfg_gone: BTreeMap<String, BTreeMap<String, u32>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catálogo: {0}")]
    Json(#[from] serde_json::Error),
}

impl Palettes {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, CatalogError> {
        Ok(serde_json::from_slice(bytes)?)
    }

    /// `cor(P, i, on)` do original: a cor `i` da paleta, na versão brilhante se `on`.
    pub fn color(&self, palette: Palette, i: u32, gloss: bool) -> &str {
        let (name, base) = match palette {
            Palette::Pal => ("PAL", &self.pal),
            Palette::Skin => ("SKIN", &self.skin),
            Palette::Eye => ("EYE", &self.eye),
            Palette::Hair => ("HAIRC", &self.hairc),
        };
        let list = if gloss { self.gloss.get(name).unwrap_or(base) } else { base };
        list.get(i as usize).or_else(|| list.first()).map_or("#000000", String::as_str)
    }

    /// Limpa um `cfg` recebido (não confiável) como `cleanCfg` do servidor: campos fora da faixa viram `None`.
    pub fn clean(&self, raw: &BTreeMap<String, u32>) -> Option<Cfg> {
        let mut out = BTreeMap::new();
        for (k, &n) in &self.cr {
            let v = raw.get(k).copied().unwrap_or(0);
            let v = self.cfg_gone.get(k).and_then(|g| g.get(&v.to_string()).copied()).unwrap_or(v);
            if v >= n {
                return None;
            }
            out.insert(k.clone(), v);
        }
        Some(Cfg(out))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Pal,
    Skin,
    Eye,
    Hair,
}

/// Visual validado: um índice por campo (`skin`, `eye`, `hs`, `hat`, `fc`...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cfg(pub BTreeMap<String, u32>);

impl Cfg {
    pub fn get(&self, k: &str) -> u32 {
        self.0.get(k).copied().unwrap_or(0)
    }
    pub fn flag(&self, k: &str) -> bool {
        self.get(k) == 1
    }
}

/// Converte `#rrggbb` (sRGB) para linear, como `rgb()` em `headMat` do original.
pub fn hex_to_linear(h: &str) -> [f64; 3] {
    let h = h.trim_start_matches('#');
    let ch = |i: usize| {
        let v = f64::from(u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0)) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    [ch(0), ch(2), ch(4)]
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::Palettes;

    pub(crate) fn palettes() -> Palettes {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        Palettes::from_catalog_json(&std::fs::read(p).unwrap()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::palettes;
    use super::*;

    #[test]
    fn loads_catalog() {
        let p = palettes();
        assert_eq!(p.skin.len(), 16);
        assert_eq!(p.cr["hs"], 34);
        assert_eq!(p.color(Palette::Skin, 0, false), "#fcc89a");
        assert_ne!(p.color(Palette::Skin, 0, true), "#fcc89a");
    }

    #[test]
    fn cleans_cfg_like_server() {
        let p = palettes();
        let mut raw = BTreeMap::new();
        raw.insert("hs".to_string(), 3);
        let c = p.clean(&raw).unwrap();
        assert_eq!(c.get("hs"), 3);
        assert_eq!(c.get("skin"), 0);
        raw.insert("hs".to_string(), 99);
        assert!(p.clean(&raw).is_none());
    }
}
