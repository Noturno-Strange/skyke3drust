# Estado da migração

Atualizado em 25/11/2026. Referência: Skyke3D 3.30.1 (commit `9d6ed71`). Nenhuma funcionalidade conta como migrada
sem teste funcional; a coluna "Validação" diz como cada item foi conferido.

## Etapas (seção 12 da especificação)

| # | Etapa | Estado |
| --- | --- | --- |
| 1 | Auditar o Skyke3D existente | feito: [auditoria/visao-geral.md](auditoria/visao-geral.md) |
| 2 | Documentar sistemas, protocolos e características visuais | feito para o núcleo: [auditoria/protocolo.md](auditoria/protocolo.md); sistemas de gameplay detalhados a cada migração |
| 3 | Ferramentas de exportação dos recursos gráficos | feito: `tools/asset-exporter` (6 mundos, céu, sprites, avatar de referência, metadados, capturas) |
| 4 | Protótipo Bevy com um cenário original | feito: os 6 mundos abrem no cliente nativo com os modelos originais |
| 5 | Materiais, iluminação e shaders toon | feito: toon, contorno, domo do céu, chão, neblina, sprites e sombras; ajustes finos na lista abaixo |
| 6 | Avatares, personalização e animações | em andamento: rosto e cabeça portados (idênticos ao navegador, pixel a pixel, em 16 casos), cores de pele, roupa e calçado, 34 cabelos e 3 chapéus (geometria exportada; cores portadas e iguais às do navegador até o último bit de f32), corpo inteiro gerado (tronco, braços, pernas e pés com bermuda e bota; `bandG`, `tubeG` e `ballsGeo` portados, iguais ao export do navegador), cores brilhantes, contornos e o emissivo dos avatares (`EMI`); boca que abre nos gestos; andar, pular, sentar e os 30 gestos e danças portados (`animA`, `DANCES`), com painel 🎭; falta o editor de visual |
| 7 | Interface original | em andamento: tela inicial (nome, mundo, bots, servidor, criar, entrar, explorar), código da sala e lotação, chat da sala e nomes sobre os avatares, com as cores e a fonte do original; faltam a barra de botões e os painéis |
| 8 | Compatibilidade com o servidor existente | núcleo feito: ticket, identidade, entrar/criar sala, posições, eventos de sala |
| 9 | Gameplay e Rhai | sandbox Rhai pronta; sistemas de gameplay pendentes |
| 10 | Áudio, voz, rádio e karaokê | pendente (protocolo de voz já decodificado) |
| 11 | Servidor Rust | pendente (o Node.js continua sendo a autoridade) |
| 12 | Testes de segurança, desempenho e compatibilidade | parcial: testes unitários, interoperabilidade com o Node, comparador visual |
| 13 | Binários Linux e Windows | CI compila e testa; publicação em Releases pendente |

## Fidelidade visual medida

Comparação automática (`tools/visual-comparator`) entre a captura do cliente web (Firefox sem janela, three.js r186)
e a do cliente nativo (Bevy 0.20, Vulkan), mesma câmera, 1280×720, sem avatares. "Pixels diferentes" conta pixels
com diferença maior que 24 em algum canal.

| Mundo | Câmera do jogo: PSNR / pixels diferentes | Visão alta: PSNR / pixels diferentes |
| --- | --- | --- |
| Amazora | 33,3 dB / 1,3 % | 36,6 dB / 0,6 % |
| Cerron | 30,7 dB / 1,8 % | 32,8 dB / 2,2 % |
| Atlanthia | 32,4 dB / 1,1 % | 36,9 dB / 0,6 % |
| Caatara | 29,9 dB / 4,2 % | 35,3 dB / 1,0 % |
| Panthara | 31,8 dB / 3,0 % | 31,2 dB / 4,2 % |
| Pamphora | 33,1 dB / 2,1 % | 35,3 dB / 1,2 % |

Histórico (câmera do jogo, Cerron): 39,7 % na primeira renderização → 16,7 % com domo do céu, chão e neblina em
sRGB → 10,6 % com sombras e cores de luz lidas sem conversão (o original desliga o `THREE.ColorManagement`, então
cores definidas por hex entram cruas no shader). Caatara: 25,9 % → 18,6 % com a mistura aditiva
(sol e brilhos) multiplicada pelo alfa, como `AdditiveBlending` do three.js. Depois, 1–4 % em todos os mundos com
duas correções: a queda da luz da fogueira que o original troca pela do three r128 (`(1 − d/alcance)^queda`) e o
sinal da normal de faces com `flatShading` (o eixo y da tela tem sentido oposto entre WGSL e GLSL). As capturas de referência variam um
pouco entre exportações por causa da cintilação da fogueira.

Para reproduzir: ver [ferramentas.md](ferramentas.md).

### Diferenças conhecidas (em ordem de impacto)

1. **Fauna 2D, partículas e efeitos animados** (`app-life.js`, `partBuild`, chamas, brasas, água): exportados só
   como geometria parada ou ausentes. Precisam de sistemas próprios (Rhai para comportamento, Rust para desenho).
2. **Chão sob a fogueira** um pouco mais escuro que no original (o círculo escuro de 2,1 m aparece levemente).
3. **Sombras**: o mapa de sombra do Bevy (1 cascata de 32 m a partir da câmera, 2048 px) em vez da caixa de 28 m que
   segue o jogador com PCF 1024 do original; o formato e a suavidade das bordas diferem um pouco.
4. Pequenas diferenças de serrilhado (MSAA 4× nativo × antialias do navegador).

## O que funciona hoje

* `skyke3d --world <mundo> --explore`: abre qualquer um dos 6 mundos com os modelos originais. Anda com WASD/setas
  (3,2 m/s), pula com E, senta com F perto de um assento e gira a câmera arrastando com o botão direito, como no
  jogo web (falta andar clicando no chão).
* `skyke3d --server http://localhost:3000 --name Ana --world cerron --bots 3`: resolve o ticket, registra ou prova a
  identidade, cria a sala no servidor Node.js atual, mostra os outros jogadores (interpolação de 230 ms) e envia a
  própria posição; `--join CODIGO` entra numa sala existente. Navegador e cliente nativo podem estar na mesma sala.
* Identidade do cliente nativo em `~/.local/share/skyke3d/online/identidade.json` (permissão 0600, só a chave
  principal; a de recuperação não é gravada).

## Limitações importantes agora

* **Código de recuperação**: o cliente nativo ainda não exibe o código de recuperação. Uma identidade criada por ele
  só existe naquele computador; se o arquivo for perdido, a conta também é. Até a tela de recuperação ser migrada,
  use o cliente nativo para testes.
* Cada jogador aparece com o próprio rosto, pele, olhos, cabelo ou chapéu, roupa, bermuda, bota e cores brilhantes
  (`cfg`). O cliente nativo envia ao servidor o visual do avatar de referência (falta o
  editor de visual).
* Interface parcial: tela inicial, chat da sala e nomes já existem; barra de botões, painéis (mochila, fazenda, culinária, amigos, ajustes) e chat global ainda não.

## Próximos passos

1. Editor de visual e boca que abre ao falar (depende da voz).
2. Fauna, partículas e efeitos (chamas, brasas, chuva, estrela cadente), com comportamentos em Rhai.
3. Resto da interface: barra de botões, painéis arrastáveis, chat global, ajustes (tema, escala, alto contraste).
4. Sistemas de culinária e fazenda do lado do cliente, a partir do catálogo já exportado (`data/catalogs/catalog.json`).
5. Voz (Opus) e áudio; mídia em WebView isolada.
6. Servidor Rust (Axum + Tokio + Rustls) validado contra a suíte de testes do servidor Node.js.
