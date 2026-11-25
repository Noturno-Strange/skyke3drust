# Protocolo do Skyke3D (versão de referência 3.30.1)

Levantado de `server.js`, `lib/validate.js`, `lib/admission.js`, `lib/identity-ws.js`, `lib/idpow.js`, `lib/ids.js`,
`assets/app-net.js`, `assets/app-voice.js` e `assets/identity.js` do repositório original. Implementado em
`crates/protocol`, `crates/security` (provas de trabalho), `crates/identity` e `crates/networking`.

## 1. Transporte

| Item | Valor |
| --- | --- |
| Conexão | Um WebSocket por jogador, na raiz do servidor (`ws(s)://host/`) |
| Cabeçalhos | `Host` e `Origin` obrigatórios e coerentes (o cliente nativo envia `Origin` = endereço do servidor) |
| Tamanho máximo do cliente | 32 KiB por mensagem (`maxPayload`), sem `permessage-deflate` |
| URL máxima | 2048 caracteres |
| Texto | JSON, sempre um objeto com o campo `t` (tipo) |
| Binário | Byte 0 = tipo: `1` estado da sala, `2` voz |

## 2. Admissão (`GET /ticket`)

1. `GET /ticket` → `{v:1, c, e, d, k}` (404 = servidor sem tickets; 429 = limite).
2. O cliente acha `n` (decimal, até 10 dígitos) tal que `SHA-256("c:n")` tenha `d` bits zero à esquerda (`d` ≤ 24).
3. Conecta em `ws(s)://host/?ticket=c.e.d.k.n`.
4. O servidor confere `k = base64url(HMAC-SHA256(segredo, "v1|c|e|d|chaveIP"))` (IPv4 ou IPv6 /64), validade
   (`e`, com folga de 120 s), a prova e o reuso.

Rust: `sky_security::pow::{solve_ticket, issue_ticket, verify_ticket}` (testado nos dois sentidos).

## 3. Identidade sem conta

| Peça | Formato |
| --- | --- |
| Chave do aparelho `k` | 32 bytes aleatórios em base64url (43 caracteres) |
| Id | `base32(SHA-256("skyke3d/amigo|"+k))`, 60 bits, 12 caracteres `[a-z2-7]` |
| Chaves | ECDSA P-256; públicas em SPKI DER base64url (`pk` principal, `rpk` recuperação) |
| Assinatura | `r‖s` cru (64 bytes) em base64url (86 caracteres), SHA-256 |
| Cargas | `"skyke3d/id/v1\n<ação>\n<id>\n…"` (ver `sky_identity::payload`) |

Fluxo de entrada:

```text
cliente → {t:'idreg', k, pk, rpk[, pow]}        (só na primeira vez)
servidor → {t:'idok', op:'register', id, gen, rgen, epoch}
          | {t:'err', scope:'idreg', code:'pow', bits}   → prova SHA-256("skyke3d/idreg|k|pk|rpk|n") e repete
cliente → {t:'auth', id, action:'join'}
servidor → {t:'challenge', id, action, nonce, exp}
cliente → {t:'proof', id, action, nonce, sig}
servidor → {t:'session', id, token, exp, gen, rgen, epoch, scope[, pending][, kp]}
cliente → {t:'accountget', token}
servidor → {t:'account', data, profile}
```

A interoperabilidade da assinatura foi conferida: uma assinatura gerada pelo `sky-identity` é aceita por
`crypto.verify(..., {dsaEncoding:'ieee-p1363'})`, o mesmo código de `lib/identity.js`.

Ainda não portados para o cliente nativo: código de recuperação (`assets/recovery.js`, formato v2), passkeys/WebAuthn,
rotação de chaves, revogação e recuperação por autorização do administrador.

## 4. Salas

```text
cliente → {t:'create', style, bots, name, cfg, token, k}
        | {t:'join', code, name, cfg, token, k}
        | {t:'resume', code, token}
servidor → {t:'joined', ver, id, code, style, host, max, players[], token, rk, self, reconnect, vopus, kar, kl, turn[, resume]}
```

* Código de sala: 10 caracteres do alfabeto `ABCDEFGHJKLMNPQRSTUVWXYZ23456789` (exibido como `ABCDE-FGHJK`).
* `players[]`/`self` (`pub` em `server.js`): `id` (`p<n>`/`b<n>`), `name`, `cfg`, `x`, `z`, `ry`, `y`, `s`, `bot`, `item`,
  `talk`, `vl`, `mk` (chave de silenciar, não revela IP), `fid`, `health`, `pp`.
* Eventos: `pj` (entrou), `pl` (saiu), `host`, `cap`, `chat`, `cfg`, `gest`, `item`, `speak`, `sys`, `err`.

## 5. Estado binário (tipo 1, servidor → cliente, 5 Hz)

```text
[u8 1] + N × [u32 id (bit 31 = bot)] [i16 x·100] [i16 z·100] [i16 giro·10000] [u8 altura·100] [u8 sentado]   (LE)
```

Arredondamento `Math.round` do JavaScript (metade para +∞); o giro é normalizado para (−π, π]. O teste
`state_matches_node_bytes` compara byte a byte com a saída do `stPack` do servidor.

O cliente desenha cada ator **230 ms no passado**, interpolando entre os dois últimos pacotes; depois de uma pausa
de mais de 350 ms repete a posição anterior 200 ms antes da nova.

## 6. Movimento (cliente → servidor)

`{t:'move', x, z, ry, y, s}` a cada 100 ms quando muda (ou a cada 2 s). O servidor limita a posição ao círculo
caminhável de raio 10,2 m em volta de (0, 1,2), `y` a 0–1,6 e `ry` a ±10. Velocidade de caminhada do cliente: 3,2 m/s.

## 7. Voz (tipo 2)

* cliente → servidor: `[u8 2][u8 codec][u8 seq][dados]`, até 50 quadros/s, só com `bufferedAmount ≤ 2048`.
* servidor → cliente: `[u8 2][u32 id][u8 codec][u8 seq][dados]` (o cliente ignora quadros com menos de 8 bytes).
* codec 1 = Opus 48 kHz mono (WebCodecs, complexidade 10, sem FEC/DTX); codec 2 = IMA-ADPCM 4 bits a 16 kHz
  (navegadores sem Opus). A sala usa Opus só quando todos têm (`vopus`).

## 8. Demais mensagens (preservadas sem perda por `ServerMsg::Other`)

Fazenda (`farmget`, `farmdo`, `farm`, `farmr`, `farmfx`), culinária (`cookget`, `cookdo`, `cookrenew`, `cook`, `cookr`,
`cookfx`), despensa (`supply`, `supplyr`), mascote (`petget`, `petadopt`, `petact`, `petdress`, `petrel`, `petsocial`,
`pet`, `petres`), saúde e medicina (`healthget`, `medcraft`, `medheal`, `health`, `healthstate`, `medr`, `medfx`),
presentes (`giftget`, `giftsee`, `giftsend`, `giftdel`, `gifts`, `giftbox`, `giftr`, `giftin`), social (`friends`,
`freq`, `facc`, `finv`, `finvr`, `fme`, `fpres`, `gchat`, `gsys`, `gn`, `pfid`, `ppid`), perfil público (`pubp`,
`plike`, `gbook`, `gblist`, `gbdel`), mídia (`video`, `vsync`, `vend`, `vstop`, `vinv`, `vbusy`, `vjoin`, `vleave`,
`vl`, `vmute`, `vlisten`, `vcap`, `vcodec`), karaokê (`kar`, `kjoin`, `kleave`, `kpass`, `ksong`, `kdur`, `kend`,
`kinv`, `ksync`), rádio (`dj`, `djstop`, `djbusy`, `djerr`), denúncias (`report`, `rep`), passkeys (`pkreg`,
`pklist`, `pkdel`, `pklogin`, `pkopt`), conta (`idrot`, `idrev`, `idrec`, `idgrant`, `idcancel`), WebRTC legado
(`sig`, `ice`), `tsync`, `bots`, `emo`.

Os formatos exatos de cada uma estão em `lib/validate.js` (`normalizeMessage`) do original; cada sistema migrado
ganhará tipos próprios em `sky-protocol` junto com testes de ida e volta.
