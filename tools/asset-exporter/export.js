#!/usr/bin/env node
/*
 * Skyke3D — exportador de mundos do cliente web (Three.js) para GLB + metadados.
 * SPDX-License-Identifier: GPL-3.0-only
 *
 * Uso:
 *   node tools/asset-exporter/export.js --original ../skyke3d --out assets/worlds [--worlds amazora,cerron] [--firefox /usr/bin/firefox]
 *
 * Como funciona: sobe um servidor HTTP local (só 127.0.0.1) que entrega os arquivos do repositório ORIGINAL sem modificá-los,
 * injeta no index.html um módulo (page/run.js) e abre o jogo num Firefox sem janela. O módulo usa o próprio buildRoom do jogo,
 * exporta cada mundo com o GLTFExporter do three.js r186 (vendor/, licença MIT) e devolve os arquivos por POST para este processo,
 * que grava em --out/<mundo>/. O repositório original só é lido.
 *
 * Saída por mundo: scene.glb (cenário), sky.glb (céu, planetas e montanhas), world.json (assentos, área caminhável, fogueira,
 * luzes, neblina, rampas toon, estatísticas) e ref-*.png (capturas de referência para o comparador visual).
 */
'use strict';
const http = require('http');
const fs = require('fs');
const path = require('path');
const os = require('os');
const vm = require('vm');
const { spawn } = require('child_process');

const args = process.argv.slice(2);
const arg = (k, d) => {
  const i = args.indexOf(k);
  return i < 0 ? d : args[i + 1];
};
const ORIG = path.resolve(arg('--original', '../skyke3d'));
const OUT = path.resolve(arg('--out', 'assets/worlds'));
const WORLDS = arg('--worlds', '');
const FIREFOX = arg('--firefox', process.env.FIREFOX || 'firefox');
const TIMEOUT = Number(arg('--timeout', '300')) * 1000;
const HERE = __dirname;

if (!fs.existsSync(path.join(ORIG, 'index.html')) || !fs.existsSync(path.join(ORIG, 'assets', 'app-scene.js'))) {
  console.error('repositório original não encontrado em ' + ORIG + ' (use --original)');
  process.exit(2);
}

// Módulo 'three' para o GLTFExporter: reexporta o THREE global do build vendorizado do jogo (mesma versão, r186).
function threeShim() {
  const sb = {};
  sb.window = sb.self = sb.globalThis = sb;
  vm.createContext(sb);
  vm.runInContext(fs.readFileSync(path.join(ORIG, 'vendor', 'three-r186.min.js'), 'utf8'), sb);
  const keys = Object.keys(sb.THREE).filter(k => /^[A-Za-z_$][\w$]*$/.test(k));
  return 'const T = window.THREE;\n' + keys.map(k => `export const ${k} = T.${k};`).join('\n') + '\n';
}

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.woff2': 'font/woff2',
  '.webmanifest': 'application/manifest+json'
};
const SAFE_NAME = /^([a-z]+\/(scene\.glb|sky\.glb|world\.json|ref-[a-z]+\.png|sprite-[0-9]{1,3}\.png)|avatar\/default\.(glb|json)|avatar\/ref-head\/(head-[0-9]{2}\.png|cases\.json)|avatar\/hair\/(hs-[0-9]{2}\.bin|hat-[0-9]\.bin|paint-[0-9]{2}\.bin|cases\.json))$/;

function inside(root, p) {
  const full = path.resolve(root, '.' + path.posix.normalize('/' + p));
  return full.startsWith(root + path.sep) ? full : null;
}

const shim = threeShim();
let done;
const finished = new Promise(r => (done = r));
const saved = [];
let errors = 0;

const server = http.createServer((q, s) => {
  const u = new URL(q.url, 'http://127.0.0.1');
  const p = u.pathname;
  if (q.method === 'POST') {
    const chunks = [];
    let size = 0;
    q.on('data', c => {
      size += c.length;
      if (size > 200 * 1024 * 1024) q.destroy();
      else chunks.push(c);
    });
    q.on('end', () => {
      const body = Buffer.concat(chunks);
      if (p === '/__export/log') {
        const t = body.toString('utf8');
        if (t.startsWith('erro')) errors++;
        console.log('[página] ' + t);
      } else if (p === '/__export/save') {
        const name = u.searchParams.get('name') || '';
        if (!SAFE_NAME.test(name)) {
          s.writeHead(400).end();
          return;
        }
        const f = path.join(OUT, name);
        fs.mkdirSync(path.dirname(f), { recursive: true });
        fs.writeFileSync(f, body);
        saved.push([name, body.length]);
      } else if (p === '/__export/done') done();
      s.writeHead(204).end();
    });
    return;
  }
  if (p === '/__export/three-shim.js') return s.writeHead(200, { 'Content-Type': TYPES['.js'] }).end(shim);
  if (p === '/__export/GLTFExporter.js' || p === '/__export/run.js') {
    const f = p.endsWith('run.js') ? path.join(HERE, 'page', 'run.js') : path.join(HERE, 'vendor', 'GLTFExporter.js');
    return s.writeHead(200, { 'Content-Type': TYPES['.js'] }).end(fs.readFileSync(f));
  }
  const file = inside(ORIG, p === '/' ? '/index.html' : p);
  if (!file || !fs.existsSync(file) || !fs.statSync(file).isFile()) return s.writeHead(404).end();
  let body = fs.readFileSync(file);
  if (file === path.join(ORIG, 'index.html')) {
    const inject =
      '<script type="importmap">{"imports":{"three":"/__export/three-shim.js"}}</script>' +
      '<script type="module" src="/__export/run.js"></script>';
    body = Buffer.from(body.toString('utf8').replace('</body>', inject + '</body>'));
  }
  s.writeHead(200, { 'Content-Type': TYPES[path.extname(file)] || 'application/octet-stream' }).end(body);
});

server.listen(0, '127.0.0.1', async () => {
  const port = server.address().port;
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'sky-export-ff-'));
  // WebGL no modo sem janela e sem pedir nada ao usuário
  fs.writeFileSync(
    path.join(profile, 'user.js'),
    [
      'user_pref("webgl.force-enabled", true);',
      'user_pref("webgl.disabled", false);',
      'user_pref("gfx.webrender.software", true);',
      'user_pref("browser.shell.checkDefaultBrowser", false);',
      'user_pref("datareporting.policy.dataSubmissionEnabled", false);',
      'user_pref("toolkit.telemetry.enabled", false);',
      'user_pref("media.autoplay.default", 5);',
      'user_pref("browser.aboutwelcome.enabled", false);'
    ].join('\n')
  );
  const url = `http://127.0.0.1:${port}/?worlds=${encodeURIComponent(WORLDS)}`;
  console.log('servindo ' + ORIG + ' em ' + url);
  const ff = spawn(FIREFOX, ['--headless', '--no-remote', '--profile', profile, '--width', '1280', '--height', '720', url], {
    stdio: ['ignore', 'ignore', 'pipe'],
    env: { ...process.env, MOZ_HEADLESS: '1' }
  });
  ff.stderr.on('data', () => {});
  const timer = setTimeout(() => {
    console.error('tempo esgotado');
    errors++;
    done();
  }, TIMEOUT);
  await finished;
  clearTimeout(timer);
  ff.kill('SIGTERM');
  server.close();
  fs.rmSync(profile, { recursive: true, force: true });
  for (const [n, b] of saved) console.log(`  ${n}  ${(b / 1024).toFixed(1)} KiB`);
  console.log(saved.length + ' arquivos em ' + OUT + (errors ? `, ${errors} erro(s)` : ''));
  process.exit(errors || !saved.length ? 1 : 0);
});
