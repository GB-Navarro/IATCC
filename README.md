# IATCC: classificador de estruturas de opções FX (MVP)

Implementação do `mvp_otc_classificador.md`: captura slices públicos do DTCC (CFTC_FX),
junta pernas por segundo + plataforma + par, calcula o teste do hedge e pede a uma IA a
estrutura do grupo segundo o brain em `brain/`. Resultado no SQLite (`mvp.db`), numa tela
estilo Bloomberg e via MCP.

## Como rodar

Guia detalhado, parte por parte, com o funcionamento interno: [howto.md](howto.md).

```
cargo run --release --bin capturar -- --minutos 60          # 1. congela data/amostra/ (horário de mercado)
cargo run --release --bin preparar                         # 2. cria mvp.db
cargo run --release --bin classificar -- --limite 20       # 3. chama a IA (só grupos pendentes)
cargo run --release --bin web                              # 4. http://127.0.0.1:3000
claude mcp add dtcc-fx -- ./target/release/mcp             # 5. (opcional) MCP; no Windows .\target\release\mcp.exe
```

Se a compilação falhar com "os error 1455" (arquivo de paginação pequeno) ou "can't find
crate for std", é falta de memória: rode `cargo clean` e compile com `cargo build --release -j 1`.

Os passos 1, 2 e 4 não precisam de chave de API. Todos os binários rodam com a pasta do
projeto como diretório de trabalho (acham `mvp.db`, `brain/` e `config/` por caminho relativo).

Variações úteis:

- `capturar -- --minutos 0 --recentes 60`: pega na hora os slices da última hora (o
  `Slice.json` guarda ~24 h).
- `capturar -- --dia 2026-09-30`: um dia inteiro pelo `Cumulative.json` (~2 mil grupos;
  classificar todos custa caro, use `--limite`).
- `preparar -- --forcar`: reconstrói o banco mesmo que já haja classificações (que são apagadas).
- `classificar -- --grupo "<id>"`, `--todos` (reclassifica tudo), `--golden` (roda a IA sobre
  os exemplos do brain em `golden/casos.jsonl` e compara, sem gravar).

## Chave de API e provider

O provider sai de `--provider` ou da variável `CLASSIFIER_PROVIDER` (padrão `anthropic`).

| Provider | Variáveis obrigatórias | Opcionais |
|---|---|---|
| `anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_MODEL` (padrão `claude-opus-5-5`) |
| `openai` | `OPENAI_API_KEY` | `OPENAI_MODEL` (padrão `gpt-6-astra`), `OPENAI_BASE_URL` |
| `jev` | `JEV_API_KEY` ou `TYPESAFE_API_KEY` | `JEV_MODEL` (padrão `jev-1.13.0`), `JEV_BASE_URL` (padrão `https://api.typesafe.ai`) |
| `devin` | `DEVIN_API_KEY`, `DEVIN_ORG_ID` | `DEVIN_API_BASE_URL`, `DEVIN_POLL_INTERVAL_SECONDS`, `DEVIN_MAX_POLLS` |

No PowerShell: `$env:ANTHROPIC_API_KEY = "sk-ant-..."` antes de `cargo run`.

## Como saber se está funcionando

- `cargo test`: inclui o caso de referência GBP/USD da seção 6 (σ, Δ e resíduos) e a regra
  do código aplicada aos 7 exemplos do brain.
- `classificar -- --golden`: a IA deve reproduzir os 7 exemplos (consistência, não acerto).
- Tela e `resumo` do MCP mostram quantas respostas da IA concordam com a regra do código.

## Organização

- `brain/`: o que a IA lê (00 a 05). `brain/_plano_v2/` guarda o brain antigo do plano v2 e
  não é carregado.
- `config/thresholds.toml`: `tau_res`, `tau_sep`, prazo mínimo e faixa de F/K.
- `config/jev_criteria.json`: critérios da pergunta choice do Jev, um por estrutura.
- `src/`: `parse.rs` (CSV → pernas), `grupos.rs`, `calculos.rs` (teste do hedge e regra do
  código), `ia.rs` + `providers/` (IA), `db.rs` (SQLite), `dtcc.rs` (captura), `bin/`.
