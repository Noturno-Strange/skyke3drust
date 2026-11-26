#!/usr/bin/env bash
# Roda a suíte de testes do servidor Node.js original contra o skyke3d-server (Rust).
#
#   tools/server-compat/run.sh [pasta-do-original] [arquivos de teste...]
#
# Copia o repositório original para uma pasta temporária (o original não é alterado), troca em tests/harness.js o
# `node server.js` pelo executável Rust e roda `node --test` nos testes que sobem o servidor.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
orig="$(cd "${1:-$here/../skyke3d}" && pwd)"
shift || true
cargo build --quiet --release -p sky-server --manifest-path "$here/Cargo.toml"
bin="$here/target/release/skyke3d-server"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
( cd "$orig" && tar --exclude=.git -cf - . ) | ( cd "$tmp" && tar -xf - )
[ -d "$tmp/node_modules/ws" ] || ( cd "$tmp" && npm install --no-audit --no-fund --silent )
python3 - "$tmp/tests" <<'PY'
import sys, glob
old = "spawn(process.execPath, ['server.js'], {"
new = "spawn(process.env.SKY_SERVER_BIN || process.execPath, process.env.SKY_SERVER_BIN ? ['--no-properties'] : ['server.js'], {"
n = 0
for p in glob.glob(sys.argv[1] + '/*.js'):
    s = open(p).read()
    if old in s:
        open(p, 'w').write(s.replace(old, new))
        n += 1
assert n >= 1, 'nenhum teste sobe o servidor: atualize o run.sh'
PY
cd "$tmp"
if [ "$#" -gt 0 ]; then files=("$@"); else mapfile -t files < <(grep -l startServer tests/*.test.js); fi
SKY_SERVER_BIN="$bin" node --test --test-concurrency=4 "${files[@]}"
