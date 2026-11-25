# Auditoria do Skyke3D original (3.30.1)

Repositório de referência: <https://github.com/Noturno-Strange/skyke3d> (somente leitura). Commit auditado: `9d6ed71`.

## Tamanho e organização

| Parte | Arquivos | Linhas (aprox.) |
| --- | --- | --- |
| Servidor (`server.js` + `lib/*.js`) | 41 | 13 000 |
| Cliente (`assets/app-*.js`, `catalog.js`, `identity.js`, `pet-art.js`…) | 31 | 9 000 (minificado à mão) |
| Páginas (`index.html`, `fazenda.html`, `profile.html`, `status.html`, `privacidade.html`) | 5 | — |
| Testes (`tests/*.test.js`) | 110 | — |
| Ferramentas (`tools/`) | 8 | — |
| Lançador desktop (`launcher/`, `desktop/src-tauri`) | — | — |

Dependências: servidor com uma só (`ws`); cliente com three.js r186 vendorizado; fonte Manrope (OFL).

## Sistemas

| Sistema | Onde está no original | Autoridade | Estado na migração |
| --- | --- | --- | --- |
| Renderização e cenas dos 6 mundos | `app-base.js`, `app-scene.js`, `app-colony.js` | cliente | **exportado + renderizado** (ver `estado-da-migracao.md`) |
| Fauna 2D, partículas, chuva, estrela cadente | `app-life.js`, `app-scene.js` (`partBuild`) | cliente | pendente |
| Avatares (13 rostos, 34 cabelos, chapéus, roupas, cores brilhantes) | `app-avatar.js`, `catalog.js` | cliente (visual) / servidor (valida `cfg`) | modelo de referência exportado; editor pendente |
| Gestos e danças | `catalog.js` (`GESTS`), `app-net.js` | servidor repassa | pendente |
| Identidade, recuperação, passkeys, Keypass | `identity.js`, `recovery.js`, `lib/identity*.js`, `lib/webauthn.js`, `lib/auth.js` | servidor | entrada (registro + prova) **funcionando** |
| Salas, bots, anfitrião, reconexão | `server.js`, `lib/room.js`, `lib/bots.js` | servidor | entrar/criar **funcionando**; reconexão pendente |
| Chat da sala e global, filtro | `app-net.js`, `lib/profanity.js`, `lib/social.js` | servidor | recebido (log); interface pendente |
| Amigos, presença, convites, notificações | `app-social.js`, `lib/social.js` | servidor | pendente |
| Perfil público, mural, curtidas | `profile.html`, `lib/pubprofile.js` | servidor | pendente |
| Culinária (182 receitas, 88 ingredientes) | `app-cook.js`, `lib/cook.js`, `catalog.js` | servidor | pendente |
| Fazenda (100 plantas, terreno, casa, safras) | `app-farm.js`, `lib/farm.js`, `fazenda.html` | servidor | pendente |
| Saúde e medicina (18 receitas) | `app-health.js`, `medicine.js`, `lib/health.js` | servidor | pendente |
| Mascote | `app-pet.js`, `pet-art.js`, `lib/pets.js`, `lib/pet-*.js` | servidor | pendente |
| Presentes | `app-gift.js`, `lib/gifts.js` | servidor | pendente |
| Conquistas e níveis | `lib/badges.js`, `lib/levels.js`, `lib/records.js` | servidor | pendente |
| Voz (Opus/ADPCM pelo WebSocket) | `app-voice.js`, `mic-worklet.js`, `lib/voicews.js` | servidor repassa | quadros binários decodificados no protocolo; áudio pendente |
| Vídeo sincronizado (até 4), karaokê, rádio | `app-media.js`, `lib/video.js`, `lib/karaoke.js`, `lib/radio.js` | servidor | pendente (WebView isolada para YouTube) |
| Música lofi e sons gerados por código | `lofi.js`, `app-sound.js` | cliente | pendente |
| Interface (painéis arrastáveis, temas, alto contraste, escala) | `index.html`, `app.css`, `app-ui.js`, `app-panels.js` | cliente | pendente |
| Moderação: denúncias, banimento, capacidade, painel `/status` | `lib/reports.js`, `lib/banmgr.js`, `lib/capacity.js`, `status.html` | servidor | pendente (permanece no Node.js) |

## Mecanismos de segurança a preservar

* Ticket de admissão com prova de trabalho amarrado ao IP (`lib/admission.js`).
* `Host`/`Origin` obrigatórios; CSP rígida nas páginas; COOP/CORP; HSTS em produção.
* Limites por IP e por ação (`hitCount`, `over`, `blocked`, `lib/action-limit.js`) e banimento progressivo.
* Validação por allowlist de cada mensagem (`lib/validate.js`), nomes reservados e homógrafos (`skeleton`).
* Identidade com chave principal + chave de recuperação, carência de recuperação, Keypass, passkeys.
* Voz repassada pelo servidor (ninguém vê o IP de ninguém), chave de silenciar derivada por HMAC.
* Antifraude de fazenda/culinária (ritmo de robô, cansaço diário, identidades em série).
* Registro de segurança (`lib/seclog.js`) e de fala (só quem e quando, nunca o áudio).

## Características visuais (ver também `docs/toon-overhaul.md` do original)

* `MeshToonMaterial` com rampa de 3 faixas (`[128,200,255]` cenário, `[176,226,255]` avatares), tingida pela cor
  de preenchimento de cada mundo (`toonTint`).
* Luz: hemisfério + direcional principal com sombra (PCF 1024, segue o jogador) + direcional de preenchimento sem
  sombra + luz pontual da fogueira (intensidade e alcance por mundo).
* Neblina linear aplicada em sRGB; céu em domo com degradê (horizonte, meio, alto); planetas, sol e brilhos como sprites.
* Chão: textura de canvas repetida 68×, que vira a cor média entre 8 e 34 m com ondulação de brilho.
* Contornos em casco invertido (cor escura do matiz do chão de cada mundo); sombras de contato em uma malha.
* Saída sRGB sem tone mapping, MSAA, câmera 60° (72° em telas estreitas), terceira pessoa 9,2 m atrás e 4,8 m acima.
* Medições do original (Chrome + SwiftShader, 1280×720): 485–538 draw calls e 140–237 mil triângulos por mundo.
