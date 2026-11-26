# Servidor do Skyke3D (`skyke3d-server`)

O servidor do jogo em Rust, num executável só, no estilo de um servidor de Minecraft: você baixa, roda numa pasta e
pronto. Ele fala o mesmo protocolo do servidor Node.js original, então jogadores do **navegador** e do **cliente
nativo** entram nas mesmas salas.

## Rodar

```bash
./skyke3d-server                 # na pasta onde quer guardar tudo
./skyke3d-server --dir /srv/sky  # ou indicando a pasta
```

Na primeira execução ele cria:

| O quê | Para quê |
| --- | --- |
| `server.properties` | as opções do servidor, com comentários |
| `data/` | identidades, nomes reservados, fazendas, culinária, mascotes, presentes, perfis, banimentos, denúncias, recordes |
| `web/` | o cliente do navegador (vem no pacote da Release); sem ela o servidor não sobe |

Depois é só abrir `http://IP-DO-SERVIDOR:3000` no navegador, ou entrar pelo cliente nativo com
`skyke3d --server http://IP-DO-SERVIDOR:3000 --name Ana`.

## `server.properties`

Cada opção é o nome da variável de ambiente do servidor original em minúsculas, com `-` no lugar de `_`. A variável de
ambiente, quando existe, tem prioridade sobre o arquivo (assim as implantações antigas continuam iguais).

| Opção | Padrão | O que faz |
| --- | --- | --- |
| `port` | 3000 | porta HTTP e WebSocket |
| `bind-host` | 0.0.0.0 | endereço de escuta |
| `public-url` | vazio | endereço público em https (produção); vazio = desenvolvimento, aberto para a rede local |
| `maxrooms` | 60 | salas (1 a 150) |
| `max-per-room` | 16 | pessoas por sala (2 a 40); bots não contam |
| `maxbots` | 200 | bots no servidor inteiro |
| `status-token` | vazio | token do painel `/status` (24 caracteres ou mais; vazio desliga o painel) |
| `ticket-mode` | vazio | `off`, `warn` ou `enforce` (vazio = enforce em produção, warn em desenvolvimento) |
| `voice-transport` | ws | `ws` (o servidor repassa a voz) ou `webrtc` (precisa de TURN) |
| `reconnect-ms` | 30000 | tempo para reconectar sem perder o lugar |
| `data-dir` | data | pasta dos dados |
| `web-root` | web | pasta do cliente do navegador |
| `ban` | 1 | banimento progressivo por abuso |
| `passkeys` | 1 | passkeys como alternativa ao código de recuperação |
| `privacy-controller`, `privacy-email` | vazio | responsável e contato mostrados em `/privacidade` |
| `motd` | Um servidor de Skyke3D | mensagem mostrada no console ao abrir |

Todas as outras variáveis do original também valem no arquivo (`turn-url`, `turn-secret`, `voice-privacy`,
`origins`, `allowed-hosts`, `trust-proxy`, `discord-report-webhook`, os arquivos `*-file` e os limites `perip`,
`perip-rate`, `global-rate`, `http-rate`, `raw-perip`…). Um arquivo com valor `0` (`farm-file=0`) fica só na memória.

## Console

Digite no terminal onde o servidor roda:

| Comando | O que faz |
| --- | --- |
| `help` | lista os comandos |
| `list` | quem está online e em que sala |
| `rooms` | salas abertas, mundo, pessoas e bots |
| `say <texto>` | aviso para todas as salas |
| `kick <nome>` | desconecta um jogador |
| `ban <nome> [minutos]` | bane o IP do jogador (padrão 60 min; o IP nunca aparece, só um ID) |
| `bans` / `pardon <id>` | banimentos ativos / remover um |
| `setmax <salas> <por sala>` | muda a capacidade na hora (e grava em `server.properties`) |
| `save-all` | grava todos os dados agora |
| `status` | as métricas do painel em JSON |
| `stop` | grava tudo e desliga (Ctrl+C também) |

## Painel `/status`

Com `status-token` definido, `http://servidor/status` mostra jogadores (agrupados por um ID do IP, nunca o IP),
denúncias com o registro de quem falou na voz, banimentos, capacidade, emblemas, renovações da chave de recuperação,
recuperação assistida e as métricas (tick, atraso, mensagens, voz, CPU, memória, rede e recordes).

## Compatibilidade

`tools/server-compat/run.sh` roda a suíte de testes do servidor Node.js original contra este executável (o original
não é alterado: os testes rodam numa cópia temporária).

```bash
tools/server-compat/run.sh ../skyke3d                       # todos os testes que sobem o servidor
tools/server-compat/run.sh ../skyke3d tests/farm-ws.test.js # um arquivo
```
