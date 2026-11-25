# Ferramentas

## Exportador de mundos (`tools/asset-exporter`)

Gera `assets/worlds/<mundo>/` e `assets/avatars/` a partir do cliente web original, sem modificá-lo.

```bash
node tools/asset-exporter/export.js --original ../skyke3d --out assets/worlds [--worlds cerron,amazora]
mv assets/worlds/avatar/* assets/avatars/ && rmdir assets/worlds/avatar
```

Requisitos: Node.js 22+ e Firefox (`--firefox` ou `FIREFOX` para outro caminho). Nenhum pacote npm é instalado.

Como funciona: um servidor HTTP local (só `127.0.0.1`) entrega os arquivos do repositório original e injeta no
`index.html` o módulo `page/run.js`, que roda no Firefox sem janela. Para cada mundo ele chama o próprio
`buildRoom` do jogo e:

* exporta `scene.glb` (cenário) e `sky.glb` (céu, planetas, montanhas) com o `GLTFExporter` do three.js r186
  (`vendor/`, licença MIT, SHA-256 em `vendor/SHA256SUMS`);
* anota cada material em `extras.sky` (`toon` com rampa de cenário ou avatar, `basic`, `outline`, `points`,
  `sky-dome` com as cores do degradê, `groundAvg` do chão, lado, transparência, neblina, cores de vértice);
* troca `ShaderMaterial` por um material básico com a anotação (o exportador do three.js descartaria a malha) e
  reduz o `userData` às anotações (o jogo guarda objetos inteiros ali);
* grava os sprites como dados + `sprite-N.png`;
* grava `world.json`: área caminhável, fogueira, 11 assentos, áreas da fauna, rampas toon tingidas, luzes
  (cor, intensidade, posição e alvo), neblina, câmeras de referência e estatísticas;
* tira `ref-jogo.png` e `ref-alto.png` (1280×720) com as câmeras fixas, sem avatares.

O avatar do menu (visual sorteado) vai para `avatar/default.glb` como modelo de referência.

## Comparador visual (`tools/visual-comparator`)

```bash
cargo run -p sky-client --bin skyke3d -- --world cerron --shot jogo --out /tmp/cerron.png
cargo run -p sky-visual-comparator -- assets/worlds/cerron/ref-jogo.png /tmp/cerron.png --diff /tmp/dif.png --min-psnr 20
```

Mostra PSNR, diferença média e porcentagem de pixels com diferença perceptível; `--diff` grava um mapa de calor
(vermelho = diferente) e `--min-psnr` faz o comando falhar abaixo do limite (para uso em CI com GPU).

## Sonda de rede (`crates/networking/examples/probe.rs`)

```bash
# no repositório original, com tudo só na memória:
PORT=3998 IDENTITY_FILE=0 PLAYER_DATA_FILE=0 NAMES_FILE=0 node server.js
# neste repositório:
cargo run -p sky-networking --example probe -- http://localhost:3998 caatara 3
```

Cria uma sala com 3 bots e mostra o que chega do servidor durante alguns segundos.

## Interoperabilidade de assinaturas

`cargo run -p sky-identity --example sign_demo` imprime chave pública e assinatura; conferir no Node.js com
`crypto.verify('sha256', dados, {key, dsaEncoding:'ieee-p1363'}, assinatura)` (o mesmo código de `lib/identity.js`).
