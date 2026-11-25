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
| 6 | Avatares, personalização e animações | em andamento: rosto/cabeça portados (idênticos ao navegador, pixel a pixel, em 16 casos) e cores de pele, roupa e calçado por jogador; faltam cabelo, chapéus, bermuda/bota, brilho, editor e animações |
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
| Amazora | 27,3 dB / 5,1 % | 30,0 dB / 2,1 % |
| Cerron | 23,8 dB / 11,1 % | 23,6 dB / 15,8 % |
| Atlanthia | 24,3 dB / 8,3 % | 25,7 dB / 9,8 % |
| Caatara | 23,6 dB / 18,6 % | 25,7 dB / 11,4 % |
| Panthara | 26,1 dB / 8,6 % | 24,6 dB / 13,0 % |
| Pamphora | 23,9 dB / 11,9 % | 22,9 dB / 17,5 % |

Histórico (câmera do jogo, Cerron): 39,7 % na primeira renderização → 16,7 % com domo do céu, chão e neblina em
sRGB → 10,6 % com sombras e cores de luz lidas sem conversão (o original desliga o `THREE.ColorManagement`, então
cores definidas por hex entram cruas no shader). Caatara: 25,9 % → 18,6 % com a mistura aditiva
(sol e brilhos) multiplicada pelo alfa, como `AdditiveBlending` do three.js. As capturas de referência variam um
pouco entre exportações por causa da cintilação da fogueira.

Para reproduzir: ver [ferramentas.md](ferramentas.md).

### Diferenças conhecidas (em ordem de impacto)

1. **Fauna 2D, partículas e efeitos animados** (`app-life.js`, `partBuild`, chamas, brasas, água): exportados só
   como geometria parada ou ausentes. Precisam de sistemas próprios (Rhai para comportamento, Rust para desenho).
2. **Luz da fogueira nos troncos**: no original, os troncos ficam bem mais laranja. A matemática do toon e da
   atenuação foi conferida linha a linha com o three.js r186; falta descobrir a origem (cintilação no momento da
   captura, brilho aditivo ou outra contribuição).
3. **Mancha escura sob a fogueira**: a malha de sombras de contato aparece mais escura que no original perto da
   fogueira.
4. **Sombras**: implementadas com o mapa de sombra do Bevy (1 cascata de 32 m a partir da câmera, 2048 px), só para
   os objetos que o original marca (`markCasters`) e os avatares. O original usa uma caixa de 28 m que segue o
   jogador, com PCF 1024: o formato e a suavidade das bordas diferem um pouco.
5. Pequenas diferenças de serrilhado (MSAA 4× nativo × antialias do navegador).

## O que funciona hoje

* `skyke3d --world <mundo>`: abre qualquer um dos 6 mundos com os modelos originais. Caminha com WASD/setas e gira a
  câmera com Q/E (3,2 m/s, mesma câmera do jogo web).
* `skyke3d --server http://localhost:3000 --name Ana --world cerron --bots 3`: resolve o ticket, registra ou prova a
  identidade, cria a sala no servidor Node.js atual, mostra os outros jogadores (interpolação de 230 ms) e envia a
  própria posição; `--join CODIGO` entra numa sala existente. Navegador e cliente nativo podem estar na mesma sala.
* Identidade do cliente nativo em `~/.local/share/skyke3d/online/identidade.json` (permissão 0600, só a chave
  principal; a de recuperação não é gravada).

## Limitações importantes agora

* **Código de recuperação**: o cliente nativo ainda não exibe o código de recuperação. Uma identidade criada por ele
  só existe naquele computador; se o arquivo for perdido, a conta também é. Até a tela de recuperação ser migrada,
  use o cliente nativo para testes.
* Cada jogador aparece com o próprio rosto, pele, olhos, camisa, calça e calçado (`cfg`), mas o formato do cabelo,
  o chapéu e a bermuda/bota ainda são os do avatar de referência; o cliente nativo envia o visual desse avatar.
* Interface parcial: tela inicial, chat da sala e nomes já existem; barra de botões, painéis (mochila, fazenda, culinária, amigos, ajustes) e chat global ainda não.

## Próximos passos

1. Portar o resto do gerador de avatar (corpo em tubos, cabelo por campo de distância e surface nets, chapéus,
   brilho), conferindo cada peça com a geometria gerada pelo navegador.
2. Fauna, partículas e efeitos (chamas, brasas, chuva, estrela cadente), com comportamentos em Rhai.
3. Resto da interface: barra de botões, painéis arrastáveis, chat global, ajustes (tema, escala, alto contraste).
4. Sistemas de culinária e fazenda do lado do cliente, a partir do catálogo já exportado (`data/catalogs/catalog.json`).
5. Voz (Opus) e áudio; mídia em WebView isolada.
6. Servidor Rust (Axum + Tokio + Rustls) validado contra a suíte de testes do servidor Node.js.
