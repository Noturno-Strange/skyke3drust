# Scripts Rhai

Implementado em `crates/scripting` (`ScriptHost`). Scripts ficam em `scripts/<área>/<nome>.rhai`.

## Quando usar Rhai

* **Sim**: regras com lógica que muda com frequência — condições de receitas, crescimento de plantas, comportamento
  de mascotes e animais, interações com objetos, eventos e clima, missões, regras de cada mundo.
* **Não**: dados puramente declarativos (nomes, cores, preços, ingredientes, configurações) ficam em `data/*.json`;
  física, renderização, rede, segurança e cálculos pesados ficam em Rust.
* Scripts rodam **por evento** ("planta regada", "receita pronta", "começou a chover"), nunca a cada quadro.

## Sandbox

| Proteção | Como |
| --- | --- |
| Sem sistema operacional | motor criado com `Engine::new_raw` + pacote de linguagem pura; sem arquivos, rede, processos, relógio |
| Sem `eval` | símbolo desativado; todo código passa pela compilação e validação |
| Sem módulos | `set_max_modules(0)` (`import` falha) |
| Operações | 50 000 por chamada (laço infinito é interrompido) |
| Profundidade | 16 chamadas aninhadas, 48 níveis de expressão |
| Memória | strings de até 4 KiB, arrays de até 1024 itens, mapas de até 256 chaves |
| Tamanho do código | 64 KiB por script |
| Nomes | `área/nome` com `[a-z0-9_-/]`, sem `..` nem caminho absoluto |
| Saída | `print`/`debug` vão para o log (`tracing`), nunca para o terminal |
| Estado | cada chamada tem escopo novo; o estado do jogo vive no Rust e entra pelos argumentos |

Os limites estão em `Limits` e são testados (`infinite_loop_is_stopped`, `deep_recursion_is_stopped`,
`huge_strings_are_stopped`, `no_eval_and_no_modules`, `rejects_bad_names_and_big_sources`).

## Versão da API

Cada script pode declarar `const API = 1;`. Um script de outra versão principal é recusado ao carregar.
Cada sistema de gameplay registra só as funções que permite (`ScriptHost::engine_mut().register_fn`), documentadas
nesta página quando forem criadas.

## Cache e recarga

Scripts são compilados uma vez (cache por nome + hash do código). Em desenvolvimento, `reload_if_changed()`
recompila só os arquivos alterados; se o novo código tiver erro, a versão anterior continua valendo.

## Autoridade

No multiplayer, scripts que decidem progresso, inventário ou recompensas rodam **só no servidor**. O cliente pode
usar scripts apenas para efeitos visuais e comportamento decorativo.
