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
| 5 | Materiais, iluminação e shaders toon | em andamento: toon, contorno, domo do céu, chão, neblina e sprites portados; faltam sombras |
| 6 | Avatares, personalização e animações | iniciado: modelo de referência exportado e desenhado; editor e animações pendentes |
| 7 | Interface original | pendente |
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
| Amazora | 24,2 dB / 32,4 % | 26,6 dB / 6,2 % |
| Cerron | 21,0 dB / 16,7 % | 19,9 dB / 18,2 % |
| Atlanthia | 21,9 dB / 44,0 % | 22,7 dB / 15,7 % |
| Caatara | 21,0 dB / 40,8 % | 22,7 dB / 14,7 % |
| Panthara | 23,3 dB / 13,2 % | 22,5 dB / 16,6 % |
| Pamphora | 22,2 dB / 31,5 % | 21,1 dB / 21,0 % |

Para reproduzir: ver [ferramentas.md](ferramentas.md).

### Diferenças conhecidas (em ordem de impacto)

1. **Sombras**: a luz principal do original projeta sombra (PCF, mapa 1024 que acompanha o jogador). Ainda não há
   sombras no material toon nativo.
2. **Fauna 2D, partículas e efeitos animados** (`app-life.js`, `partBuild`, chamas, brasas, água): exportados só
   como geometria parada ou ausentes. Precisam de sistemas próprios (Rhai para comportamento, Rust para desenho).
3. **Mancha escura sob a fogueira**: a malha de sombras de contato aparece mais escura que no original perto da
   fogueira. Investigar a mistura das cores de vértice com alfa e a ordem de desenho dos transparentes.
4. **Luz da fogueira nos troncos**: os troncos recebem menos laranja que no original; conferir a cintilação
   (`FIREK`, intensidade no momento da captura) e se a fogueira precisa de mais de uma luz.
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
* Todos os atores usam o mesmo avatar de referência (exportado do menu do jogo web); o visual de cada jogador
  (`cfg`) ainda não é aplicado.
* Sem interface: chat, nomes, painéis e menus ainda não existem no cliente nativo (eventos vão para o log).

## Próximos passos

1. Sombras direcionais no material toon (mapa de sombra próprio ou integração com as sombras do Bevy).
2. Exportar as partes do avatar separadas (rostos, cabelos, chapéus, roupas) e montar o avatar pelo `cfg`, com
   comparação visual por peça.
3. Fauna, partículas e efeitos (chamas, brasas, chuva, estrela cadente), com comportamentos em Rhai.
4. Interface com Bevy UI a partir de `index.html`/`app.css` (temas claro, escuro e alto contraste, escala).
5. Catálogo declarativo (`catalog.js` → `data/*.json`) e sistemas de culinária e fazenda do lado do cliente.
6. Voz (Opus) e áudio; mídia em WebView isolada.
7. Servidor Rust (Axum + Tokio + Rustls) validado contra a suíte de testes do servidor Node.js.
