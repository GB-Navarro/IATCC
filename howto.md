# Como rodar e entender o MVP

Guia prático do classificador de estruturas de opções FX: o que cada parte faz, como rodar
cada uma sozinha, como rodar tudo junto e como conferir que está funcionando. A
especificação completa está em `mvp_otc_classificador.md`, na pasta acima deste projeto.

## Sumário

1. [Visão geral](#1-visão-geral)
2. [Antes de começar](#2-antes-de-começar)
3. [Rodar tudo em conjunto](#3-rodar-tudo-em-conjunto)
4. [Cada parte separadamente](#4-cada-parte-separadamente)
5. [Como funciona por dentro](#5-como-funciona-por-dentro)
6. [Ciclo de melhoria do brain](#6-ciclo-de-melhoria-do-brain)
7. [Problemas comuns](#7-problemas-comuns)

---

## 1. Visão geral

```
 DTCC (arquivos públicos, CFTC_FX)
        │  1. capturar   → data/amostra/*.zip + manifest.json   (amostra congelada)
        ▼
        │  2. preparar   → mvp.db: pernas, grupos, números do teste do hedge, regra do código
        ▼
        │  3. classificar → brain/*.md + grupo em JSON → IA → resposta gravada em mvp.db
        ▼
        ├──► 4. web  (tela estilo Bloomberg em http://127.0.0.1:3000)
        └──► 5. mcp  (um assistente consulta e pede classificações)
```

Divisão de trabalho:

| Quem | Faz | Não faz |
|---|---|---|
| Código | Lê os arquivos, junta pernas, calcula todos os números (deltas, resíduos) e aplica a regra do brain como linha de base | Não envia a sua resposta à IA |
| IA | Lê o brain e os números prontos e decide a estrutura, explicando o porquê | Não faz contas, não inventa campos |
| Brain (`brain/*.md`) | Ensina o que cada estrutura é, o que o dado mostra e quando responder "Ambiguous" | Não guarda código |

O que precisa de chave de API: **só a classificação pela IA** (passo 3, F8/F9 na tela e
`classificar_grupo` no MCP). Captura, preparação, tela, consultas do MCP e testes rodam sem chave.

## 2. Antes de começar

**Pasta de trabalho.** Todos os comandos rodam a partir da pasta do projeto, porque os
caminhos padrão (`data/amostra`, `mvp.db`, `brain`, `config`, `static`) são relativos:

```powershell
cd "C:\Users\gsilv\Brain\Rust Lab\IATCC\IATCC"
```

**Compilar.** Rust estável com edição 2024. Nesta máquina a compilação em paralelo esgota a
memória, então use um job só:

```powershell
cargo build --release -j 1
```

Se aparecer "os error 1455" ou "can't find crate for std", rode `cargo clean` e repita.
Depois do build, `cargo run --release --bin <nome>` não recompila nada.

**Chave de API (só para a IA).** Escolha um provider e defina as variáveis na sessão do
PowerShell antes de rodar:

| Provider | Obrigatórias | Opcionais |
|---|---|---|
| `anthropic` (padrão) | `ANTHROPIC_API_KEY` | `ANTHROPIC_MODEL` (padrão `claude-opus-5-5`) |
| `openai` | `OPENAI_API_KEY` | `OPENAI_MODEL` (padrão `gpt-6-astra`), `OPENAI_BASE_URL` |
| `jev` | `JEV_API_KEY` ou `TYPESAFE_API_KEY` | `JEV_MODEL` (padrão `jev-1.13.0`), `JEV_BASE_URL` |
| `devin` | `DEVIN_API_KEY`, `DEVIN_ORG_ID` | `DEVIN_API_BASE_URL`, `DEVIN_POLL_INTERVAL_SECONDS`, `DEVIN_MAX_POLLS` |

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
# para outro provider: --provider openai (ou jev, devin) no comando,
# ou $env:CLASSIFIER_PROVIDER = "openai"
```

A variável vale só para aquele terminal. Se abrir outro terminal, defina de novo.

---

## 3. Rodar tudo em conjunto

### 3.1 Sem chave (dados, números e tela)

```powershell
cargo test -j 1                                                   # 20 testes passam
cargo run --release --bin capturar -- --minutos 0 --recentes 60   # última hora de slices
cargo run --release --bin preparar                                # cria mvp.db
cargo run --release --bin web                                     # abra http://127.0.0.1:3000
```

Na tela você já vê os grupos, as pernas, o teste do hedge e a coluna CÓDIGO (a regra do
brain aplicada em código). A coluna IA fica com "—" até haver classificações.

### 3.2 Com chave (fluxo completo)

Terminal 1:

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."
cargo run --release --bin classificar -- --golden      # 7 chamadas: a IA reproduz os exemplos do brain?
cargo run --release --bin classificar -- --limite 20   # classifica 20 grupos pendentes e grava
```

Terminal 2 (pode ficar aberto o tempo todo):

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-..."   # necessário só para F8/F9 na tela
cargo run --release --bin web
```

Recarregue a página para ver as novas classificações. Tela e `classificar` podem rodar ao
mesmo tempo sobre o mesmo `mvp.db`.

Opcional, o MCP no Claude Code (seção [4.6](#46-mcp)): o cliente inicia o servidor sozinho,
você só registra uma vez.

### 3.3 Começar do zero com outra amostra

```powershell
cargo run --release --bin capturar -- --dia 2026-09-30 --diretorio data/dia-30
cargo run --release --bin preparar -- --amostra data/dia-30 --banco dia30.db
cargo run --release --bin web -- --banco dia30.db
```

Cada amostra pode ter seu próprio banco. Todos os comandos aceitam `--banco`.

---

## 4. Cada parte separadamente

### 4.1 Testes

```powershell
cargo test -j 1
```

| Teste | O que garante |
|---|---|
| `calculos::caso_gbp_reproduz_o_teste_do_documento` | O caso real GBP/USD da seção 6 do MVP: σ, Δ e os três resíduos batem com até 5e-6 |
| `calculos::exemplos_1_e_4...`, `formas_do_arquivo_02`, `motivos_da_secao_6` | Tabela de decisão, nomes das formas e motivos de ausência do teste |
| `golden::regra_do_codigo_reproduz_os_exemplos_do_brain` | A regra do código acerta os 7 exemplos de `golden/casos.jsonl` |
| `parse::linhas_reais_viram_pernas...` | Parser com linhas reais do DTCC: prêmio, moeda-base, lado, capado, NEWT |
| `db::...` | Gravação, leitura, filtros, pendentes, proteção contra apagar classificações |
| `dtcc::...` | Índice do DTCC e detecção de amostra alterada |
| `providers::...` | Pedido e resposta de Anthropic, OpenAI, Jev e Devin com rede simulada |

### 4.2 `capturar`: congelar a amostra

**O que faz.** Lê o índice público do DTCC (`Slice.json`, que lista as últimas ~24 h de
slices de ~10 s), baixa os zips de `CFTC_FX` pela URL do índice e grava em `data/amostra/`.
Cada download entra no `manifest.json` com o `sliceId`, a URL, os horários e o SHA-256. Uma
falha num slice não interrompe os outros: ele fica de fora e é tentado na próxima volta.

**Opções:**

| Comando | Efeito |
|---|---|
| `capturar -- --minutos 60` | Captura ao vivo por 60 min: só slices publicados a partir de agora, consultando o índice a cada 15 s. Rode em horário de mercado (13h–17h UTC, 10h–14h em Brasília) |
| `capturar -- --minutos 0 --recentes 60` | Sem esperar: pega os slices da última hora já listados no índice |
| `capturar -- --minutos 30 --recentes 30` | As duas coisas: 30 min passados e 30 min ao vivo |
| `capturar -- --dia 2026-09-30` | O arquivo cumulativo de um dia inteiro (`Cumulative.json`). O dia D fica disponível depois da meia-noite UTC de D+1 |
| `--diretorio <pasta>` | Grava em outra pasta (padrão `data/amostra`) |

**Como conferir.** `data/amostra/manifest.json` lista os arquivos; a saída mostra
`captura: N novo(s) slice(s), M no manifesto` a cada volta. Rodar de novo na mesma pasta
acrescenta à amostra sem baixar repetidos.

**Tamanho.** Uma hora de slices rende algumas centenas de grupos; um dia inteiro, ~2.300.
Para começar, uma hora basta.

### 4.3 `preparar`: pernas, grupos e números

**O que faz**, em ordem:

1. Confere o SHA-256 de cada zip contra o manifesto. Se algum arquivo mudou, para com erro:
   a amostra é fixa.
2. Lê cada CSV pelo nome das colunas e fica só com `Action type = NEWT`.
3. Dá nome a cada perna pela coluna `UPI FISN`: NDF, Forward, Opção vanilla, Opção NDO ou
   Opção exótica. O resto (swaps etc.) é ignorado.
4. Junta as pernas em grupos por **mesmo segundo + mesma plataforma + mesmo par**. Opções
   entram sempre; NDF e Forward só com `Package indicator` verdadeiro; grupos sem opção são
   descartados.
5. Calcula os números de cada grupo e o teste do hedge (seção [5.2](#52-teste-do-hedge)).
6. Aplica a regra do código (seção [5.3](#53-regra-do-código)).
7. Reconstrói o `mvp.db`.

```powershell
cargo run --release --bin preparar
```

**Saída** (real, para o dia 30/09/2026 inteiro; trecho):

```
56625 pernas NEWT lidas -> 4105 pernas úteis em 2283 grupos (65 com teste do hedge) em mvp.db
      116  Ambiguous
       35  Risk reversal
     1367  Opção única
      321  Straddle
      ...
```

**Proteção.** Se o banco já tem classificações da IA, `preparar` recusa reconstruir (elas
custaram dinheiro). Use `-- --forcar` para apagá-las e reconstruir mesmo assim.

**Opções:** `--amostra`, `--banco`, `--config` (thresholds), `--forcar`.

### 4.4 `classificar`: a IA

**O que faz.** Para cada grupo, monta o pedido com o brain inteiro (os 6 arquivos `.md`
concatenados, enviados como instrução de sistema e em cache) e o grupo em JSON. A IA responde
num formato fixo (JSON Schema): `estrutura`, `alternativas`, `regra`, `confianca` e
`justificativa`. A resposta é validada e gravada com o modelo, o hash do brain, o hash da
configuração e o hash da entrada.

**O que a IA recebe:** as pernas (família, lado, moeda-base, strike, vencimento, notionais,
prêmio, câmbio, capado, Package indicator) e os números do grupo (contagens e `teste_hedge`
ou o motivo de não haver). **Não** recebe a regra do código nem a linha CSV bruta.

**Opções:**

| Comando | Efeito |
|---|---|
| `classificar` | Todos os grupos **pendentes**: sem classificação para este brain, provider e modelo |
| `classificar -- --limite 20` | No máximo 20 grupos nesta execução (controle de custo) |
| `classificar -- --grupo "2026-09-30T13:45:56Z\|TSEF\|BRL-USD"` | Um grupo específico, mesmo que já classificado |
| `classificar -- --todos` | Reclassifica todos, pendentes ou não |
| `classificar -- --golden` | Roda a IA sobre os 7 exemplos de `golden/casos.jsonl` e compara com a resposta esperada, **sem gravar** |
| `--provider openai` | Troca o provider (ou `CLASSIFIER_PROVIDER`) |

**Formato da saída** (números ilustrativos):

```
provider anthropic · modelo claude-opus-5-5
20 grupo(s) a classificar
2026-09-30T13:45:56Z|TSEF|BRL-USD: Risk reversal (03/R4, alta) · entrada 1450 (cache lido 1900, cache gravado 0), saída 610
...
20 classificado(s), 0 falha(s)
```

`cache lido` maior que zero a partir da segunda chamada indica que o brain está em cache.
Falhas de um grupo (erro HTTP, JSON inválido) são mostradas e o lote continua.

**Custo.** Alguns centavos de dólar por grupo. Sem `--limite`, ele classifica todos os
pendentes, o que num dia inteiro passa de 2 mil chamadas.

**Detalhes do provider Anthropic.** `max_tokens` 8000 (o modelo raciocina antes de responder
e esse raciocínio conta no limite). Para `claude-opus-5-5` vai junto `fallbacks: "default"`:
se um filtro de segurança recusar o pedido, a própria API tenta outro modelo. Uma recusa
final aparece como "a IA recusou a classificação" e nada é gravado.

### 4.5 `web`: a tela

```powershell
cargo run --release --bin web            # http://127.0.0.1:3000
cargo run --release --bin web -- --bind 127.0.0.1:8080 --banco dia30.db
```

Abrir a tela não exige chave: o provider só é criado quando você pede uma classificação.

**O que a tela mostra:**

- **Barra de título:** janela da amostra, total de grupos, quantos a IA classificou, quantos
  concordam com o código e o provider configurado.
- **Linha de comando (topo):** digite um par (`BRL`) ou uma estrutura (`RISK REVERSAL`,
  `OPÇÃO ÚNICA`) e tecle Enter. O filtro também pode vir na URL:
  `http://127.0.0.1:3000/#RISK%20REVERSAL`.
- **Lista (esquerda):** HORA, PLAT, PAR, OPT (opções), HDG (hedges), CÓDIGO e IA.
- **Detalhe (direita):**
  - PERNAS: `VAN CALL USD 158.75 30.24M USD 2026-10-29 PKG`. Hedges em ciano, `+` após o
    notional capado, PKG quando há Package indicator.
  - TESTE DO HEDGE: resíduos, separações, limiares, deltas, vols, forward, H/N; ou
    "AUSENTE" com o motivo.
  - IA: estrutura, regra, confiança, alternativas e justificativa.
  - CÓDIGO: a regra do código e ✓ CONCORDA ou ✗ DISCORDA.
  - HISTÓRICO, quando o grupo tem mais de uma classificação.

**Cores:** âmbar = estrutura; amarelo = Ambiguous; cinza = não é estrutura (opção única,
fracionamento, não identificado); ciano = hedge; vermelho = IA discorda do código.

**Teclas:**

| Tecla | Ação |
|---|---|
| ↑ ↓ e Enter | Navegar na lista e abrir o detalhe (ou clique na linha) |
| F1 | Voltar à lista |
| F2 | Mostrar/ocultar o texto do brain e seu hash |
| F8 | Classificar o grupo selecionado |
| F9 | Classificar em sequência todos os grupos da lista ainda sem IA (pede confirmação) |
| ESC | Cancela o F9 em andamento; senão, limpa o filtro |

Erros (chave ausente, HTTP da IA) aparecem na linha de status, acima das teclas.

**API HTTP** (a tela usa estas rotas; dá para chamar com `curl` ou o navegador):

| Rota | Retorna |
|---|---|
| `GET /api/info` | Resumo da amostra, provider e lista de estruturas |
| `GET /api/grupos?par=&estrutura=&limite=` | Grupos com a última classificação |
| `GET /api/grupos/{id}` | Grupo completo, classificação atual e histórico |
| `POST /api/grupos/{id}/classificar` | Classifica agora e devolve a resposta |
| `GET /api/brain` | Texto e hash do brain |

O `{id}` precisa estar codificado na URL (o `|` vira `%7C`).

### 4.6 `mcp`

**O que é.** Um servidor MCP: um assistente de IA (Claude Code, Claude Desktop) o inicia e
conversa com ele por JSON-RPC 2.0, uma mensagem por linha, pelo stdin e stdout. Assim você
pergunta em linguagem natural, por exemplo "quais grupos de BRL ficaram ambíguos?", e o
assistente chama as ferramentas abaixo. Você não deixa o servidor rodando: o cliente o inicia
quando precisa.

**Ferramentas:**

| Ferramenta | Argumentos | O que faz | Chave? |
|---|---|---|---|
| `listar_grupos` | `par`, `estrutura`, `limite` (opcionais; padrão 100) | Lista grupos com horário, plataforma, par, nº de opções e hedges, regra do código, última classificação da IA e se concordam. `par` aceita um trecho (`BRL`); `estrutura` casa com a IA ou com o código, sem diferenciar maiúsculas nem acentos | Não |
| `ver_grupo` | `grupo_id` | O grupo exatamente como a IA o recebe, a regra do código e o histórico de todas as classificações (modelo, hash do brain, resposta) | Não |
| `classificar_grupo` | `grupo_id` | Chama a IA, grava a resposta e a devolve | Sim, no ambiente do servidor |
| `resumo` | nenhum | Total de grupos, janela da amostra, contagem por estrutura (IA e código), classificados, concordâncias e lista de discordâncias | Não |

O `grupo_id` tem o formato `2026-09-30T13:45:56Z|TSEF|BRL-USD` (horário, plataforma, par).

**Teste rápido sem cliente** (no PowerShell, depois do build):

```powershell
'{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | .\target\release\mcp.exe
'{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"resumo","arguments":{}}}' | .\target\release\mcp.exe
```

**Registrar no Claude Code** (uma vez). Use caminhos absolutos para o servidor achar o banco
e o brain de onde quer que seja iniciado:

```powershell
$P = "C:\Users\gsilv\Brain\Rust Lab\IATCC\IATCC"
claude mcp add dtcc-fx -e ANTHROPIC_API_KEY=sk-ant-... -- "$P\target\release\mcp.exe" --banco "$P\mvp.db" --brain "$P\brain" --config "$P\config\thresholds.toml" --jev-criteria "$P\config\jev_criteria.json"
```

Sem chave, omita `-e ANTHROPIC_API_KEY=...`: as consultas funcionam e `classificar_grupo`
devolve um erro claro. Para outro provider, acrescente `--provider openai` (e a variável dele
com `-e`). Confira com `/mcp` dentro do Claude Code.

---

## 5. Como funciona por dentro

### 5.1 Pernas, lado e moeda-base

- **Família**, pelo início do `UPI FISN`: `NA/Fwd NDF` → NDF; `NA/Fwd XXX YYY` e
  `NA/FX Fwd Nstd` → Forward; `NA/O NDO` → Opção NDO; `NA/O Van` → Opção vanilla;
  `NA/O Dig`, `NA/O Bar`, `NA/O Targ`, `NA/FX O Nstd`, `NA/FX Op` → Opção exótica.
- **Par:** as moedas de `Notional currency-Leg 1` e `Leg 2`, em ordem alfabética (`BRL-USD`).
- **Lado** (`call_ccy`): `Call currency`; se vazia, a outra moeda do par em relação a
  `Put currency`. As palavras "Call" e "Put" do FISN não são usadas, porque erram com
  frequência.
- **Moeda-base** (a moeda em que o strike é cotado por unidade): primeiro testa qual perna de
  notional, multiplicada pelo strike, reproduz a outra. Isso é necessário porque a notação
  "X/Y" do DTCC não é consistente entre reportantes (há `BRL/USD` com strike 5,05, que é
  USD-base). Sem essa evidência, vale a primeira moeda do par do strike e, depois, a do
  `Exchange rate basis`.
- **Capado:** notional terminado em `+` é só um limite inferior; a perna é marcada.

### 5.2 Teste do hedge

Quem negocia uma call e uma put juntas às vezes faz também um forward/NDF para neutralizar o
risco cambial. O DTCC não diz quem comprou e quem vendeu, mas publica o tamanho do hedge (H),
e esse tamanho depende da direção:

| Hipótese | Hedge previsto (na moeda-base) |
|---|---|
| Strangle (mesma direção) | D_mesma = \|Δc·Nc + Δp·Np\| |
| Risk reversal (direções opostas) | D_opostas = Δc·Nc + \|Δp\|·Np |
| Collar (notional cheio) | D_cheio = (Nc + Np) / 2 |

Para cada hipótese, `res_X = |D_X − H| / (Nc + Np)` (0 = bate perfeito). As separações
`sep = (D_opostas − D_mesma)/(Nc+Np)` e `sep_cheio = |D_cheio − D_opostas|/(Nc+Np)` dizem se
as hipóteses são distinguíveis.

Os deltas vêm de Black-76: F é o câmbio do hedge (invertido se cotado ao contrário), T são os
dias até o vencimento / 365, o preço por unidade sai do prêmio, σ é encontrado por bisseção
e Δ = Φ(d1) para a call e Φ(d1) − 1 para a put.

**Quando há teste:** exatamente 2 opções vanilla ou NDO, em lados opostos, com o mesmo
vencimento e um hedge. **Quando não há**, o motivo vai para a IA. Os principais:

| Motivo | Significado |
|---|---|
| não se aplica: ... | A forma do grupo não é a de 2 opções em lados opostos com o mesmo vencimento |
| nenhum hedge no grupo | Nenhum NDF/Forward com Package indicator no mesmo segundo |
| notional capado | Alguma perna tem notional com `+` |
| prêmio vazio ou numa moeda fora do par | Não dá para calcular o preço |
| σ sem solução | O prêmio é incompatível com Black-76 entre σ = 0,0001 e 3 |
| T menor que 2 dias | Vencimento curto demais |
| F/K fora de 0.7 a 1.4 | Strike muito longe do forward |

Os limiares ficam em `config/thresholds.toml` (`tau_res`, `tau_sep`, `min_t_days` e a faixa
de F/K), e não no brain, para que números escritos em prosa não mudem entre versões.

### 5.3 Regra do código

`calculos.rs` aplica em código o que o brain ensina, como linha de base para comparar com a
IA. O resultado aparece na coluna CÓDIGO e **nunca é enviado à IA**.

1. **Forma** (arquivo 02): pelo número de opções, lados, strikes e vencimentos. Opção única,
   Straddle, Spread vertical, Calendar spread, Spread diagonal, Fracionamento de notional,
   Lados opostos com vencimentos diferentes, Seagull, Strip/Strap, Ratio spread, Iron condor,
   Butterfly, Condor, Programa de hedge ou Não identificado.
2. **Tabela do arquivo 03**, só para 2 opções em lados opostos, strikes diferentes e mesmo
   vencimento. Vale a primeira regra verdadeira:

| Regra | Condição | Resultado |
|---|---|---|
| R1 | teste do hedge ausente | Ambiguous |
| R2 | sep < tau_sep | Ambiguous (deltas pequenos) |
| R3 | res_opostas ≤ tau e res_cheio ≤ tau e sep_cheio < tau_sep | Ambiguous (risk reversal ou collar) |
| R4 | res_opostas ≤ tau e res_mesma > tau | Risk reversal |
| R5 | res_mesma ≤ tau e res_opostas > tau | Strangle |
| R6 | res_mesma ≤ tau e res_opostas ≤ tau | Ambiguous (deltas pequenos) |
| R7 | res_cheio ≤ tau e sep_cheio ≥ tau_sep | Collar |
| R8 | nenhuma das anteriores | Ambiguous (hedge não explicado) |

### 5.4 Brain

Arquivos em `brain/`, concatenados em ordem alfabética e enviados inteiros a cada chamada:

| Arquivo | Conteúdo |
|---|---|
| `00_papel.md` | Papel da IA e regras gerais (não fazer contas, não inventar campos, Ambiguous é válido) |
| `01_o_que_o_dado_mostra.md` | O que o DTCC publica e o que não publica |
| `02_estruturas.md` | Definição de cada estrutura |
| `03_teste_do_hedge.md` | A tabela R1–R8 |
| `04_formato_da_resposta.md` | Os campos da resposta |
| `05_exemplos.md` | 7 exemplos com a resposta esperada |

`brain/_plano_v2/` guarda o brain antigo do plano v2 e **não** é carregado (só os `.md` do
primeiro nível entram). `config/jev_criteria.json` traduz as estruturas em critérios para o
provider Jev, que recebe o brain dentro do estado.

### 5.5 Golden cases

`golden/casos.jsonl` tem os 7 exemplos do arquivo 05, cada um com o grupo exatamente como a
IA o recebe e a resposta esperada. São usados duas vezes: pelo teste da regra do código
(`cargo test`) e por `classificar -- --golden`, que mede se a IA é consistente com o próprio
brain. Isso mede consistência, não acerto no mundo real, porque a IA viu esses exemplos.

### 5.6 Banco (`mvp.db`)

| Tabela | Conteúdo |
|---|---|
| `grupos` | id (`horário\|plataforma\|par`), números (JSON com contagens e teste do hedge), `regra_codigo` e a regra que a decidiu |
| `pernas` | Campos normalizados de cada perna e a linha CSV original em JSON (auditoria) |
| `classificacoes` | Uma linha por resposta da IA: provider, modelo, hash do brain, da configuração e da entrada, resposta, resposta bruta da API, latência |
| `journal` | Eventos de reconstrução e de classificação |

Dá para abrir com qualquer cliente SQLite (DB Browser for SQLite, extensão do VS Code).

---

## 6. Ciclo de melhoria do brain

1. Na tela, procure linhas com a IA em vermelho (discorda do código), ou veja
   `discordancias` no `resumo` do MCP.
2. Abra o grupo, leia a justificativa da IA e os números do teste do hedge.
3. Edite o arquivo do brain que causou a divergência. Se mudar uma regra, atualize também
   `05_exemplos.md` e, se fizer sentido, `golden/casos.jsonl` e `calculos.rs`.
4. `cargo run --release --bin classificar -- --golden` para ver se os exemplos continuam
   reproduzidos.
5. `cargo run --release --bin classificar -- --limite 20`: como o hash do brain mudou, os
   grupos voltam a ser pendentes. Compare no HISTÓRICO do detalhe.

---

## 7. Problemas comuns

| Sintoma | Causa e solução |
|---|---|
| `mvp.db não existe; rode antes ... preparar` | O passo 2 ainda não rodou, ou o comando está em outra pasta. Rode da pasta do projeto ou passe `--banco` |
| `manifest.json não existe; rode antes ... capturar` | Falta o passo 1 (ou `--amostra` aponta para outra pasta) |
| `... mudou desde a captura: o SHA-256 não bate` | Um zip da amostra foi alterado. Capture de novo numa pasta limpa |
| `o banco já tem N classificação(ões)` | `preparar` protege as respostas pagas. Use `--forcar` ou outro `--banco` |
| `ANTHROPIC_API_KEY não está definida` | Defina a variável no mesmo terminal (ou `-e` no `claude mcp add`) |
| `Anthropic HTTP 401` | Chave inválida |
| `Anthropic HTTP 400` | Pedido recusado pela API; a mensagem diz o campo. Se citar `fallbacks`, use um modelo sem fallback via `ANTHROPIC_MODEL` |
| `atingiu max_tokens antes de terminar o JSON` | A resposta não coube em 8000 tokens; reclassifique o grupo com `--grupo` |
| `captura: 0 novo(s) slice(s)` por muito tempo | Fora do horário de mercado o fluxo é pequeno. Use `--recentes` ou `--dia` |
| Lista vazia na tela | Filtro ativo (ESC limpa) ou banco vazio (`preparar`) |
| Tela não muda depois de editar `static/index.html` | Cache do navegador: Ctrl+F5 |
| Compilação falha com "os error 1455" | Falta de memória: `cargo clean` e `cargo build --release -j 1` |
