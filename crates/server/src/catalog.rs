//! Catálogo do jogo (`assets/catalog.js` e `assets/medicine.js`, exportados em `data/catalogs/catalog.json`), embutido
//! no executável: o servidor valida itens, receitas, plantas, gestos e o visual com as mesmas tabelas do cliente.

use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Debug)]
pub struct Catalog {
    pub raw: Value,
    /// Campos do visual e quantas opções cada um tem, na ordem do original.
    pub cr: Vec<(String, i64)>,
    pub cfg_gone: HashMap<String, Map<String, Value>>,
    pub styles: Vec<String>,
    pub st_legacy: HashMap<String, String>,
    pub max_name: usize,
    pub max_bots: i64,
    pub max_emo: i64,
    pub items: HashSet<String>,
    pub food: HashSet<String>,
    pub recipe_items: HashSet<String>,
    pub ing_ids: HashSet<String>,
    pub cook_methods: HashSet<String>,
    pub cook_slots: usize,
    pub farm_cells: i64,
    pub crops: HashMap<String, Value>,
    pub recipes: HashMap<String, Value>,
    pub gests: HashSet<String>,
    pub sound_gests: HashSet<String>,
    pub medicine: HashMap<String, Value>,
}

fn ids(v: &Value, filter: impl Fn(&Value) -> bool) -> HashSet<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|x| filter(x))
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect()
}

fn by_id(v: &Value) -> HashMap<String, Value> {
    v.as_array().into_iter().flatten().filter_map(|x| Some((x["id"].as_str()?.to_owned(), x.clone()))).collect()
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

impl Catalog {
    pub fn from_json(raw: Value) -> Self {
        let i = |k: &str| raw[k].as_i64().unwrap_or(0);
        Self {
            cr: raw["CR"]
                .as_object()
                .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(1))).collect())
                .unwrap_or_default(),
            cfg_gone: raw["CFG_GONE"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_object()?.clone()))).collect())
                .unwrap_or_default(),
            styles: raw["ST"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default(),
            st_legacy: raw["ST_LEGACY"]
                .as_object()
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
                .unwrap_or_default(),
            max_name: raw["LIMITS"]["name"].as_u64().unwrap_or(16) as usize,
            max_bots: raw["LIMITS"]["bots"].as_i64().unwrap_or(20),
            max_emo: raw["EMO"].as_array().map_or(0, |a| a.len() as i64 - 1),
            items: ids(&raw["ITEMS"], |_| true),
            food: ids(&raw["ITEMS"], |x| truthy(&x["food"])),
            recipe_items: ids(&raw["ITEMS"], |x| truthy(&x["rc"])),
            ing_ids: ids(&raw["ING"], |_| true),
            cook_methods: raw["COOK_METHODS"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default(),
            cook_slots: i("COOK_SLOTS") as usize,
            farm_cells: i("FARM_W") * i("FARM_H"),
            crops: by_id(&raw["CROPS"]),
            recipes: by_id(&raw["RECIPES"]),
            gests: ids(&raw["GESTS"], |_| true),
            sound_gests: ids(&raw["GESTS"], |x| truthy(&x["sfx"])),
            medicine: by_id(&raw["MEDICINE"]["MEDICINES"]),
            raw,
        }
    }

    /// O catálogo que vem dentro do executável.
    pub fn embedded() -> &'static Catalog {
        static C: OnceLock<Catalog> = OnceLock::new();
        C.get_or_init(|| {
            let raw: Value =
                serde_json::from_str(include_str!("../../../data/catalogs/catalog.json")).unwrap_or(Value::Null);
            Catalog::from_json(raw)
        })
    }

    /// Nome antigo de mundo → atual (`styleId`).
    pub fn style_id(&self, s: &str) -> String {
        self.st_legacy.get(s).cloned().unwrap_or_else(|| s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_tables() {
        let c = Catalog::embedded();
        assert_eq!(c.cr.len(), 22);
        assert_eq!(c.cr[0].0, "skin");
        assert_eq!(c.styles.len(), 6);
        assert_eq!(c.items.len(), 206);
        assert_eq!(c.farm_cells, 64);
        assert_eq!(c.medicine.len(), 18);
        assert_eq!(c.style_id("proxima"), "cerron");
    }
}
