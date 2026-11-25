#!/usr/bin/env node
/*
 * Skyke3D — exporta o catálogo do jogo original (assets/catalog.js e assets/medicine.js) para JSON.
 * SPDX-License-Identifier: GPL-3.0-only
 *
 * Uso: node tools/catalog-export/export.js --original ../skyke3d --out data/catalogs/catalog.json
 *
 * Só dados: funções (vivid, recipeKey, farmOpen) e índices derivados (ITEM, RECIPE, CROP, INGR, GEST, RECIPE_KEY,
 * FARM_SOURCES) ficam de fora; o Rust reconstrói os índices ao carregar. O repositório original só é lido.
 */
'use strict';
const fs = require('fs');
const path = require('path');
const args = process.argv.slice(2);
const arg = (k, d) => {
  const i = args.indexOf(k);
  return i < 0 ? d : args[i + 1];
};
const ORIG = path.resolve(arg('--original', '../skyke3d'));
const OUT = path.resolve(arg('--out', 'data/catalogs/catalog.json'));
const CAT = require(path.join(ORIG, 'assets', 'catalog.js'));
const MED = require(path.join(ORIG, 'assets', 'medicine.js'));
const SKIP = new Set(['ITEM', 'RECIPE', 'CROP', 'INGR', 'GEST', 'RECIPE_KEY', 'FARM_SOURCES']);
const out = { v: 1, source: 'skyke3d ' + require(path.join(ORIG, 'package.json')).version };
for (const [k, v] of Object.entries(CAT)) if (typeof v !== 'function' && !SKIP.has(k)) out[k] = v;
const med = {};
for (const [k, v] of Object.entries(MED)) if (typeof v !== 'function') med[k] = v;
out.MEDICINE = med;
const json = JSON.stringify(out, null, 1);
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, json + '\n');
console.log(OUT + ': ' + Object.keys(out).length + ' chaves, ' + (json.length / 1024).toFixed(1) + ' KiB');
