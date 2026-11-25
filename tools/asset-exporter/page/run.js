/*
 * Skyke3D — exportador de mundos (roda DENTRO do cliente web original, num Firefox sem janela).
 * SPDX-License-Identifier: GPL-3.0-only
 *
 * Os scripts do jogo (assets/app-*.js) são scripts comuns: o que declaram no nível de cima (buildRoom, room, rig, scene, cam,
 * renderer, seats, obst, FOOT, WALK, TG, TGS, OLC, hemi, dirL, fillL...) fica no escopo global e este módulo enxerga.
 * Para cada mundo: monta o cenário com o próprio buildRoom do jogo, anota os materiais (toon, contorno, básico) nos `extras`
 * do glTF, exporta GLB do cenário e do céu, grava os metadados de jogo e tira capturas de referência com câmeras fixas.
 */
import { GLTFExporter } from '/__export/GLTFExporter.js';

const send = async (name, body, type) => {
  const r = await fetch('/__export/save?name=' + encodeURIComponent(name), { method: 'POST', headers: { 'Content-Type': type }, body });
  if (!r.ok) throw new Error('falha ao salvar ' + name);
};
const log = m => fetch('/__export/log', { method: 'POST', body: String(m) });
const frame = () => new Promise(r => requestAnimationFrame(() => r()));
const hex = c => '#' + c.getHexString();
const lin = c => [c.r, c.g, c.b];
const ramp = t => {
  const d = t.image.data;
  return [0, 1, 2].map(i => [d[i * 4], d[i * 4 + 1], d[i * 4 + 2]]);
};

// Câmeras de referência (as mesmas no comparador visual): "jogo" = câmera de terceira pessoa com o avatar no ponto de entrada
// (0, 5) e giro 0 (assets/app-ui.js); "alto" = visão geral.
const SHOTS = {
  jogo: { pos: [0, 4.8, 14.2], look: [0, 1.2, 2], fov: 60 },
  alto: { pos: [0, 30, 45], look: [0, 0, 0], fov: 60 }
};

function materialInfo(m) {
  const base = {
    side: m.side === THREE.BackSide ? 'back' : m.side === THREE.DoubleSide ? 'double' : 'front',
    transparent: !!m.transparent,
    opacity: m.opacity,
    depthWrite: m.depthWrite,
    fog: m.fog !== false,
    vertexColors: !!m.vertexColors,
    blending: m.blending === THREE.AdditiveBlending ? 'additive' : m.blending === THREE.NormalBlending ? 'normal' : String(m.blending)
  };
  if (m.isMeshToonMaterial) {
    const info = { kind: 'toon', ramp: m.gradientMap === TG ? 'avatar' : 'scene', flat: !!m.flatShading, emissive: hex(m.emissive), ...base };
    // chão (groundMat em app-scene.js): longe da câmera a textura vira a cor média (linear) com uma ondulação suave
    if (m.customProgramCacheKey && m.customProgramCacheKey() === 'ground' && typeof GAVG === 'object' && GAVG)
      info.groundAvg = GAVG.clone().convertSRGBToLinear().toArray();
    return info;
  }
  if (m.isMeshBasicMaterial) return { kind: m.side === THREE.BackSide && m.color.equals(OLC) ? 'outline' : 'basic', ...base };
  if (m.isMeshStandardMaterial) return { kind: 'standard', ...base };
  if (m.isPointsMaterial) return { kind: 'points', size: m.size, sizeAttenuation: m.sizeAttenuation, ...base };
  if (m.isSpriteMaterial) return { kind: 'sprite', ...base };
  if (m.isShaderMaterial) {
    // domo do céu (assets/app-base.js): degradê horizonte → meio → alto, cores já em sRGB gravadas direto
    const u = m.uniforms || {};
    if (u.top && u.hor && u.mid) return { kind: 'sky-dome', top: u.top.value.toArray(), hor: u.hor.value.toArray(), mid: u.mid.value.toArray(), ...base };
    return { kind: 'shader', ...base };
  }
  return { kind: m.type, ...base };
}

/** Anota materiais e objetos; devolve estatísticas. */
function annotate(root) {
  const st = { meshes: 0, instanced: 0, instances: 0, tris: 0, points: 0, sprites: 0, lines: 0, kinds: {} };
  root.traverse(o => {
    const mats = o.material ? (Array.isArray(o.material) ? o.material : [o.material]) : [];
    for (const m of mats) {
      m.userData.sky = materialInfo(m);
      st.kinds[m.userData.sky.kind] = (st.kinds[m.userData.sky.kind] || 0) + 1;
    }
    if (o.isInstancedMesh) {
      st.instanced++;
      st.instances += o.count;
    }
    if (o.isMesh) {
      st.meshes++;
      const g = o.geometry,
        n = g.index ? g.index.count / 3 : g.attributes.position.count / 3;
      st.tris += n * (o.isInstancedMesh ? o.count : 1);
    }
    if (o.isPoints) st.points++;
    if (o.isSprite) st.sprites++;
    if (o.isLine) st.lines++;
    // dados de jogo que o próprio objeto carrega (contorno, colisão etc.): só o que é JSON simples
    const ud = {};
    for (const [k, v] of Object.entries(o.userData)) if (k !== 'sky' && v !== null && ['number', 'string', 'boolean'].includes(typeof v)) ud[k] = v;
    if (o.isMesh || o.isGroup) {
      // sombras: markCasters só marca objetos pequenos perto do chão; o cenário quase todo recebe
      if (o.castShadow) ud.cs = 1;
      if (o.receiveShadow) ud.rs = 1;
    }
    // partes do avatar que o cliente nativo troca pelo visual de cada jogador
    if (o.isMesh && typeof HM !== 'undefined' && HM && o.material === HM) ud.part = 'hair';
    if (o.isMesh && typeof HTM !== 'undefined' && HTM && o.material === HTM) ud.part = 'hat';
    if (o.isMesh && typeof HOL !== 'undefined' && [...HOL.values()].includes(o.material)) ud.part = 'hair-outline';
    if (o.isInstancedMesh) ud.instanced = o.count;
    if (o.isPoints) ud.points = true;
    if (o.isSprite) ud.sprite = true;
    o.userData.skyObj = Object.keys(ud).length ? ud : undefined;
  });
  return st;
}

/** Copia sem sprites (o glTF não tem sprite; o cliente nativo recria os efeitos) e sem ajudantes invisíveis. */
function exportable(root) {
  const out = [];
  root.traverse(o => {
    if (o.isSprite) out.push(o);
  });
  out.forEach(o => (o.visible = false));
  return () => out.forEach(o => (o.visible = true));
}

// O jogo guarda referências a objetos, funções e estado em userData; o GLTFExporter copiaria tudo para os `extras`.
// Durante a exportação, objetos e materiais levam só as anotações do Skyke3D (skyObj / sky); depois tudo volta.
function slimUserData(root) {
  const saved = [];
  const slim = (x, key) => {
    saved.push([x, x.userData]);
    const v = x.userData && x.userData[key];
    x.userData = v ? { [key === 'skyObj' ? 'sky' : key]: v } : {};
  };
  root.traverse(o => {
    slim(o, 'skyObj');
    const mats = o.material ? (Array.isArray(o.material) ? o.material : [o.material]) : [];
    for (const m of mats) if (!saved.some(([x]) => x === m)) slim(m, 'sky');
  });
  return () => saved.forEach(([x, u]) => (x.userData = u));
}

// O GLTFExporter descarta malhas com ShaderMaterial; elas vão com um material básico que carrega a anotação (sky-dome etc.).
function stubShaders(root) {
  const swapped = [];
  root.traverse(o => {
    if (o.isMesh && o.material && o.material.isShaderMaterial) {
      const info = o.material.userData.sky || materialInfo(o.material);
      const stub = new THREE.MeshBasicMaterial({ side: o.material.side });
      stub.userData.sky = info;
      swapped.push([o, o.material]);
      o.material = stub;
    }
  });
  return () => swapped.forEach(([o, m]) => (o.material = m));
}

async function glb(obj) {
  const ex = new GLTFExporter();
  const restoreSh = stubShaders(obj),
    restore = exportable(obj),
    restoreUd = slimUserData(obj);
  try {
    return await ex.parseAsync(obj, { binary: true, onlyVisible: true, maxTextureSize: 2048 });
  } finally {
    restoreUd();
    restore();
    restoreSh();
  }
}

async function shot(name) {
  const r = renderer,
    av = typeof me === 'object' && me && me.g ? me.g : null,
    was = av ? av.visible : false;
  if (av) av.visible = false; // só o cenário: avatares têm comparação própria
  r.setPixelRatio(1);
  r.setSize(1280, 720, false);
  for (const [k, s] of Object.entries(SHOTS)) {
    cam.fov = s.fov;
    cam.aspect = 1280 / 720;
    cam.updateProjectionMatrix();
    cam.position.set(...s.pos);
    cam.lookAt(...s.look);
    r.render(scene, cam);
    const blob = await new Promise(res => r.domElement.toBlob(res, 'image/png'));
    await send(name + '/ref-' + k + '.png', blob, 'image/png');
  }
  if (av) av.visible = was;
}

// Sprites (planetas, sol, brilhos): o glTF não tem sprite. Vão como dados + PNG; o cliente nativo desenha quads virados
// para a câmera com o mesmo tamanho, opacidade e mistura (normal ou aditiva).
async function sprites(k, roots) {
  const list = [],
    texIds = new Map();
  for (const root of roots)
    root.updateMatrixWorld(true),
      root.traverse(o => {
        if (!o.isSprite || !o.visible) return;
        const m = o.material,
          p = new THREE.Vector3(),
          q = new THREE.Quaternion(),
          sc = new THREE.Vector3();
        o.matrixWorld.decompose(p, q, sc);
        let tex = -1;
        if (m.map && m.map.image) {
          if (!texIds.has(m.map.image)) texIds.set(m.map.image, texIds.size);
          tex = texIds.get(m.map.image);
        }
        list.push({
          root: root === rig ? 'sky' : 'scene',
          pos: p.toArray(),
          scale: [sc.x, sc.y],
          center: [o.center.x, o.center.y],
          rotation: m.rotation || 0,
          color: hex(m.color),
          opacity: m.opacity,
          additive: m.blending === THREE.AdditiveBlending,
          depthTest: m.depthTest,
          depthWrite: m.depthWrite,
          fog: m.fog !== false,
          sizeAttenuation: m.sizeAttenuation !== false,
          renderOrder: o.renderOrder || 0,
          tex
        });
      });
  for (const [img, i] of texIds) {
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    c.getContext('2d').drawImage(img, 0, 0);
    const blob = await new Promise(r => c.toBlob(r, 'image/png'));
    await send(k + '/sprite-' + i + '.png', blob, 'image/png');
  }
  return list;
}

function meta(k) {
  const fogc = scene.fog ? scene.fog.color : null;
  return {
    v: 1,
    world: k,
    source: 'skyke3d ' + (window.SKY_VER || '') + ' / three r' + THREE.REVISION,
    units: 'metros; y para cima; mesmo sistema de coordenadas do cliente web',
    walk: { ...WALK },
    fire: obst ? { ...obst } : null,
    seats: (seats || []).map(s => ({ ...s })),
    foot: (FOOT || []).map(f => ({ ...f })),
    toon: { scene: ramp(TGS), avatar: ramp(TG), outline: hex(OLC) },
    lights: {
      hemi: { sky: hex(hemi.color), ground: hex(hemi.groundColor), intensity: hemi.intensity },
      key: { color: hex(dirL.color), intensity: dirL.intensity, position: dirL.position.toArray(), target: dirL.target.position.toArray(), castShadow: dirL.castShadow },
      fill: { color: hex(fillL.color), intensity: fillL.intensity, position: fillL.position.toArray(), target: fillL.target.position.toArray() },
      fire: fireL ? { color: hex(fireL.color), intensity: fireL.intensity, distance: fireL.distance, decay: fireL.decay, position: fireL.position.toArray() } : null
    },
    fog: scene.fog ? { color: hex(fogc), linear: lin(fogc), near: scene.fog.near, far: scene.fog.far } : null,
    background: scene.background && scene.background.isColor ? hex(scene.background) : null,
    renderer: {
      toneMapping: renderer.toneMapping,
      exposure: renderer.toneMappingExposure,
      outputColorSpace: renderer.outputColorSpace,
      shadowMapSize: dirL.shadow ? dirL.shadow.mapSize.x : 0
    },
    shots: SHOTS
  };
}

async function main() {
  const want = new URLSearchParams(location.search).get('worlds');
  const worlds = want ? want.split(',') : Object.keys(ST);
  await log('mundos: ' + worlds.join(', '));
  for (const k of worlds) {
    buildRoom(k);
    for (let i = 0; i < 4; i++) await frame();
    await shot(k);
    const m = meta(k);
    m.stats = { room: annotate(room), sky: annotate(rig) };
    m.sprites = await sprites(k, [room, rig]);
    await send(k + '/world.json', JSON.stringify(m, null, 1), 'application/json');
    await send(k + '/scene.glb', await glb(room), 'model/gltf-binary');
    await send(k + '/sky.glb', await glb(rig), 'model/gltf-binary');
    await log(k + ': ok ' + JSON.stringify(m.stats.room));
  }
  // Avatar do menu (visual sorteado) como modelo de referência até a personalização ser migrada (sky-avatars).
  if (typeof me === 'object' && me && me.g) {
    const g = me.g,
      pos = g.position.clone(),
      rot = g.rotation.y;
    g.position.set(0, 0, 0);
    g.rotation.y = 0;
    g.updateMatrixWorld(true);
    annotate(g);
    await send('avatar/default.glb', await glb(g), 'model/gltf-binary');
    await send('avatar/default.json', JSON.stringify({ v: 1, cfg: { ...cfg } }, null, 1), 'application/json');
    g.position.copy(pos);
    g.rotation.y = rot;
  }
  // Geometria do cabelo (34 estilos) e dos chapéus: depende só do estilo; a cor é aplicada depois por triângulo
  // (paintHair). Binário: 'SKYH', versão, nTri, nTriLo, depois em f32: pos[9n], normal[9n], centro[3n], jitter[n],
  // pos_lo[9m], onrm_lo[9m]. O cliente nativo pinta com o port de col0/paintHair (crates/avatars/src/hair.rs).
  if (typeof hairGeo === 'function') {
    const pack = (shape, lo, normalAttr) => {
      const A = shape.g.attributes,
        n = shape.JT.length,
        m = lo.g.attributes.position.count / 3,
        head = new Uint32Array([0x48594b53, 1, n, m]),
        parts = [A.position.array, A[normalAttr].array, shape.CE, shape.JT, lo.g.attributes.position.array, lo.g.attributes.onrm.array];
      return new Blob([head, ...parts.map(a => new Float32Array(a))], { type: 'application/octet-stream' });
    };
    const base = { hair: 0, skin: 0, rb: 0, tb: 0, hc: 0, hb: 0 };
    for (let hs = 0; hs < HS.length; hs++) {
      LINS++;
      try {
        const sh = hairGeo({ ...base, hs });
        await send('avatar/hair/hs-' + String(hs).padStart(2, '0') + '.bin', pack(sh.shape, sh.mkLo(), 'normal'), 'application/octet-stream');
      } finally {
        LINS--;
      }
    }
    for (let h = 1; h < HATS.length; h++) {
      LINS++;
      try {
        const sh = hatGeo(h);
        sh.shape.JT.fill(1);
        await send('avatar/hair/hat-' + h + '.bin', pack(sh.shape, sh.mkLo(), 'snrm'), 'application/octet-stream');
      } finally {
        LINS--;
      }
    }
    // cores de referência: paintHair do jogo para alguns visuais (o port em Rust tem de dar o mesmo)
    const P = [
      [{ hs: 0, hair: 3 }], [{ hs: 3, hair: 7, skin: 4 }], [{ hs: 7, hair: 12, rb: 1, skin: 9 }], [{ hs: 20, hair: 5 }],
      [{ hs: 24, hair: 1, skin: 12, tb: 1 }], [{ hs: 25, hair: 20 }], [{ hs: 31, hair: 8, skin: 2 }], [{ hs: 33, hair: 30, rb: 1 }],
      [{ hat: 1, hc: 6 }], [{ hat: 2, hc: 13, hb: 1 }], [{ hat: 3, hc: 25 }]
    ];
    const cases = [];
    for (let i = 0; i < P.length; i++) {
      const c = { ...base, ...P[i][0] };
      LINS++;
      try {
        const sh = c.hat ? hatGeo(c.hat) : hairGeo(c);
        if (c.hat) sh.shape.JT.fill(1);
        sh.setC(c);
        const K = paintHair(sh.shape, sh.col);
        const name = 'paint-' + String(i).padStart(2, '0') + '.bin';
        await send('avatar/hair/' + name, new Blob([K], { type: 'application/octet-stream' }), 'application/octet-stream');
        cases.push([name, c]);
      } finally {
        LINS--;
      }
    }
    await send('avatar/hair/cases.json', JSON.stringify(cases), 'application/json');
  }
  // Texturas de cabeça geradas pelo próprio headMat do jogo, para conferir o port em Rust (crates/avatars/src/head.rs).
  if (typeof headMat === 'function') {
    const base = { skin: 0, eye: 0, hair: 0, hs: 0, hat: 0, hc: 0, fc: 0, tb: 0, eb: 0, rb: 0 };
    const C = [
      [{}, 0], [{ fc: 1, skin: 3, eye: 2, hair: 5, hs: 3 }, 0], [{ fc: 2, skin: 7, hs: 5 }, 0], [{ fc: 3, eye: 9, hs: 24 }, 0],
      [{ fc: 4, skin: 12, hs: 25, hair: 9 }, 0], [{ fc: 5, eb: 1, eye: 4 }, 0], [{ fc: 6, tb: 1, skin: 2 }, 0], [{ fc: 7, hs: 7 }, 0],
      [{ fc: 8, hs: 31, rb: 1, hair: 12 }, 0], [{ fc: 9, hs: 11, eye: 6 }, 0], [{ fc: 10, hs: 16, skin: 10 }, 0],
      [{ fc: 11, hs: 17, skin: 14 }, 0], [{ fc: 12, eye: 12, hs: 27 }, 0], [{ fc: 0, hat: 2, hc: 5 }, 0], [{ fc: 0 }, 1], [{ fc: 6, eb: 1, hat: 1 }, 2]
    ];
    const cases = [];
    for (let i = 0; i < C.length; i++) {
      const [d, mv] = C[i],
        c = { ...base, ...d },
        m = headMat(c, mv),
        img = m.map.image,
        cv = document.createElement('canvas');
      cv.width = img.width;
      cv.height = img.height;
      cv.getContext('2d').drawImage(img, 0, 0);
      const name = 'head-' + String(i).padStart(2, '0') + '.png';
      await send('avatar/ref-head/' + name, await new Promise(r => cv.toBlob(r, 'image/png')), 'image/png');
      cases.push([name, c, mv]);
    }
    await send('avatar/ref-head/cases.json', JSON.stringify(cases), 'application/json');
  }
  await fetch('/__export/done', { method: 'POST' });
}
addEventListener('error', e => log('erro: ' + e.message + ' ' + (e.filename || '') + ':' + (e.lineno || '')));
addEventListener('unhandledrejection', e => log('erro: ' + (e.reason && (e.reason.stack || e.reason.message) || e.reason)));
(document.readyState === 'complete' ? Promise.resolve() : new Promise(r => addEventListener('load', r)))
  .then(() => new Promise(r => setTimeout(r, 500)))
  .then(main)
  .catch(e => log('erro: ' + (e.stack || e.message)).then(() => fetch('/__export/done', { method: 'POST' })));
