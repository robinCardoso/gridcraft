# Plano de desempenho do cálculo — concluir ~800k sem “travar”

> Escopo: **velocidade e conclusão** do recálculo (F9 / Automatic / load) em
> **qualquer** workbook Excel com centenas de milhares de fórmulas pesadas
> (`SE`/`LET`/`ÍNDICE`/`CORRESP`/`PROCV`/tabelas estruturadas). O arquivo de
> referência do usuário (abas tipo `ORCAMENTO` / `LISTA_PRECO` / `COTACAO`) é só
> **corpus de medição** — não define o desenho das otimizações.
>
> Responsividade da UI (nunca bloquear a thread da interface) já está coberta por
> [`plano-responsividade.md`](./plano-responsividade.md). Este documento **não reescreve**
> essa história: assume jobs, `CalcHooks`, deferral e índices *IFS já feitos, e foca no
> que ainda falta para o job **terminar em tempo útil** e o progresso **não parecer preso**.

Última análise: **2026-10-23**. Sintoma de referência: barra
`Calculando… 42% — 366938 / 793554` (screenshot).

---

## 0. Regra de ouro — otimizações universais

**Todo aprimoramento de desempenho é genérico** e vive no engine / `calc` / funções
(`MATCH`, `XLOOKUP`, `COUNTIF*`, `LET`, grafo, jobs). Vale para **qualquer** `.xlsx`.

| Proibido | Obrigatório |
|---|---|
| `if sheet_name == "COTACAO"` (ou qualquer nome de aba) | Índices e caches baseados em **forma da fórmula** / AST / tipo de lookup |
| Hardcode de workbook, caminho de arquivo, título de tabela “só deste cliente” | Workloads de exemplo só em **docs, fixtures e reteste manual** |
| Atalhos que só aceleram uma planilha conhecida | Paridade: mesmo caminho de código para todos os livros |

**“Padrão COTACAO” (legado neste repo) = workload de exemplo**, não special-case:

- `LET` aninhado + igualdade `coluna_tabela = código` (ThisRow) + `MATCH`/`INDEX` em tabelas;
- lookup repetido por linha sobre a mesma coluna de preço/lista.

Quando o texto antigo dizia “otimizar o padrão COTACAO”, lia-se (errado) “otimizar a
aba/arquivo COTACAO”. A leitura correta: **acelerar esses padrões de fórmula no
motor**, em qualquer arquivo que os use. Preferir o nome:

> **padrões de fórmula universais** (ex.: tabelas + `LET` + `MATCH` / first-true).

---

## 1. Sintoma e métricas atuais

| Sinal | Interpretação |
|---|---|
| `366938 / 793554` | Progresso **real** do motor (`done` / `graph.len()`), não um placeholder travado em 0. |
| `42%` na barra | A UI mapeia `done/total` para no máximo **90%** até o fim do job (`jobs.rs`: `* 0.9`). `0,462 × 0,9 ≈ 0,42` — bate. |
| N ≈ **793k** fórmulas | ~2× o caso `NOVA LISTA…` (~350k) que já levou ~12 s em release após índice `MATCH`. Escala linear ingenua → dezenas de segundos; com `LET`+array por linha → minutos. |
| Cálculo em job | Se a janela ainda responde (ribbon, cancelar, scroll), **não é freeze da UI** — é job lento / platô de progresso. |
| Fórmula típica (corpus de exemplo) | `SE(ORCAMENTO[@Referencia]=""; … SE(…Linha…); …)` + tabelas de preço — **padrão universal** `LET` + coluna=código + `MATCH`/`INDEX` (ver rodada LET em `plano-responsividade`). |

**Estado herdado (não refazer):** ver §4 e o handoff de `plano-responsividade.md`.

---

## 2. Como medir — freeze UI vs job lento

Antes de otimizar, separar **três falhas** com o mesmo relato verbal (“travou”):

| Tipo | Como detectar | O que fazer |
|---|---|---|
| **A. UI congelada** | Frame watchdog > 100 ms contínuos; Windows “Não está respondendo”; Cancel não pinta | Regressão de responsividade → voltar a `plano-responsividade` / freeze guard |
| **B. Progresso parado, job vivo** | Contagem `done` estática por > N segundos; CPU de 1 thread ~100%; Cancel ainda funciona | Fórmula(s) monstro sem yield fino **ou** lote de 4096 muito caro |
| **C. Job lento mas progresso sobe** | `done` sobe; ETA linear; UI fluida | Falta throughput (índices, cache, paralelismo, subset dirty) |

### 2.1 Receitas rápidas (manual)

1. **Durante o platô em ~42%:** clicar Cancel. Se a barra some em **< 1 s**, o caminho cooperativo está vivo (tipo B/C). Se nada muda, investigar deadlock / UI (tipo A).
2. **Gerenciador de tarefas:** 1 núcleo saturado + RAM estável → eval single-thread saudável mas lento. RAM subindo sem teto → materialização de arrays / cache de intervalos.
3. **CLI / release:** `GRIDCRAFT_CALC_STATS=1` (histograma por fórmula + caches) e/ou `GRIDCRAFT_PROFILE=1` (tempos de passada) no motor; cronometrar F9 no corpus denso.
4. **Painel** Ctrl+Alt+Shift+P: confirmar que frames da UI ficam baixos enquanto o job roda.

### 2.2 Instrumentação desejada (1ª sprint — ver §7)

| Métrica | Onde | Uso |
|---|---|---|
| Histograma de **µs por fórmula** (p50/p95/p99/max) | `PassHost::compute` | Achatar “uma fórmula de 30 s” |
| Tempo por **função de topo** (`LET`, `MATCH`, `INDEX`, `SE`, `SUMIF*`) | `Evaluator::call` (amostra) | Rankear hipóteses |
| Hits/miss do `range_array` e do índice de igualdade | `PassHost` | Ver se o cache estoura (`RANGE_CACHE_CELLS = 1,5 M`) |
| Intervalo entre `hooks.report` | Callback em `jobs.rs` | Quantificar “barra parada” |
| Pico de RSS do processo do job | OS / log | Decidir orçamento de cache |

Critério de leitura: se **p99 ≫ p50**, o problema não é “800k × custo médio” — é cauda (fórmulas monstro ou miss de índice).

---

## 3. Hipóteses ranqueadas — “travou” em ~42%

Ordem por **probabilidade × impacto** no corpus denso ~800k (mesmo padrão de fórmula
aparece em qualquer livro com tabelas + lookup por linha):

| # | Hipótese | Por que encaixa o 42% | Evidência a procurar |
|---|---|---|---|
| **H1** | **Eval single-thread + fórmulas caras por linha** (`LET` com `col = codigo`, `MATCH(TRUE, achou, 0)`, `INDEX` em tabela) | Progresso avança em degraus; em trechos densos parece parado. 42% é só o meio do grafo. | p99 alto; CPU 1 core; Cancel ok |
| **H2** | **Progresso só a cada 4096 fórmulas no loop externo** (`run_scoped`); dentro de **uma** fórmula, `results.len()` não sobe até ela terminar | Uma fórmula que materializa array grande “segura” a barra. Relatos de “congelou no mesmo %”. | Gap longo entre reports com `done` igual; max(compute) absurdo |
| **H3** | **Miss / estouro do cache de intervalos** (`RANGE_CACHE_CELLS` / 128 entradas) → rematerializar a mesma coluna milhares de vezes | Meio do pass quando o working set de intervalos cresce (várias abas de lista/preço) | Miss rate ↑ após ~40% |
| **H4** | **Comparação de coluna inteira / tabela grande sem índice** (boolean array + `MATCH`) | Cada linha refaz O(n) sobre a coluna de lookup; 800k × n explode | Perfil em `MATCH`/`=` sobre `Value::Array` |
| **H5** | **Pressão de memória / paging** (snapshot `Arc<Workbook>` + `results` + arrays) | SO “congela” tudo; progresso irregular | RSS alto, disco 100% |
| **H6** | **CF / pintura** reavaliando regras caras no viewport enquanto o job corre | UI “pesada”, não o numerador 366938 | Frame ms alto só ao scroll; calc thread ok |
| **H7** | **Deadlock / hang real** (Mutex em hooks, panic engolido, job sem report) | Raro após fix de cancel/progresso | Cancel morto; done eterno; thread parada |
| **H8** | **Spill / dynamic arrays** desligando caches (correção intencional) | Se o livro tiver spills, caminho lento garantido | `graph.dynamic` não vazio; caches off |
| **H9** | **`Expr::Struct` marcado `dynamic`** → `dirty_closure` inclui **todo** `graph.dynamic` em qualquer edit | Clear de ~13 células na coluna Referencia dispara ~600k | Contagem de `graph.dynamic` ≈ nº de fórmulas com structured ref; dirty ≫ seeds |

**Nota “GIL”:** não há GIL de Python. O equivalente prático é **um worker de cálculo** + Mutex curto nos hooks de progresso. Paralelismo exige partição topológica (§5.3), não “soltar o GIL”.

---

## 4. Ganhos já feitos (baseline — não refazer)

Referência detalhada: [`plano-responsividade.md`](./plano-responsividade.md) (Fases 0–5 + rodadas).

| Área | O quê |
|---|---|
| Jobs | F9 / cascata grande / estrutural → thread + barra + Cancel |
| Progresso / cancel | Report cedo (0/N), a cada 4096 no loop, sample a cada 256 em `compute`; Cancel via hooks |
| Grafo | Rebuild **incremental** no F9 |
| Índices | `MATCH`/`VLOOKUP` exact; `COUNTIFS`/`SUMIFS` igualdade (~75× medido) — **genéricos**, não por aba |
| Cache | `range_array` por passada; bindings `LET` com invariância (fix nested `variant_names`) |
| Automatic | Sem truncate >1000; deferral / `recalc_pending` / `recalc_cascade` dirty |
| Undo | Diff / COW; undo sem rebuild O(n) |
| UI | Viewport prioritário no `run_scoped`; CF cache por revisão; Fase 4 frame budget |
| Structured refs no grafo | ✅ `precedents` resolve `Expr::Struct` → `Area` (ThisRow/coluna/Headers/…) quando a tabela é conhecida; `dynamic` só se irresolvível (H9) |
| Progresso incremental | ✅ denominador = `|dirty|` na cascata Automatic (full continua `graph.len()`) |

Qualquer sprint nova deve **medir contra este baseline**, não reimplementar job ou índice *IFS.

---

## 5. Roadmap priorizado

Prioridade: índices genéricos (`MATCH` / `XLOOKUP` / `COUNTIF*`), yield/progresso,
paralelismo topológico, cache `LET` correto (**já feito**), dirty subset — tudo em
`engine` / `calc` / `functions`. Zero special-case por nome de aba ou arquivo (§0).

### 5.1 Quick wins (dias)

| ID | Ação | Efeito esperado | Risco |
|---|---|---|---|
| Q1 | ✅ **Yield/progresso por tempo** (~75 ms nativo; `PassHost::coop` + `run_scoped`) | Barra deixa de parecer morta; Cancel mais previsível | Baixo |
| Q2 | ✅ **Histograma + top-K** sob `GRIDCRAFT_CALC_STATS` (stderr ao fim do pass) | Transforma H1–H4 em dados | Baixo |
| Q3 | **Índice `COUNTIF`/`SUMIF`/`AVERAGEIFS`/`MAXIFS`/`MINIFS`** (extensão do *IFS) | Throughput em **qualquer** livro que use essas formas | Médio (paridade) |
| Q4 | **Reusar índice de lookup para `MATCH(TRUE, array_bool, 0)` / first-true** quando o array veio de `col = escalares` | Acelera o padrão universal `achou`+`MATCH` (tabelas + igualdade) | Médio |
| Q5 | **Não invalidar job por scroll/paint**; garantir CF lazy só no viewport (auditar hot path) | Tipo A/H6 | Baixo |

### 5.2 Médio prazo (1–2 sprints)

| ID | Ação | Efeito esperado |
|---|---|---|
| M1 | **Chunked write-back** + opcional aplicação parcial no viewport (já citado como pendência antiga na Fase 3) | Feedback visual cedo sem esperar 800k |
| M2 | **Dirty subset primeiro**: Automatic / seeds / aba ativa antes do full book; full só sob F9 explícito ou idle | “Concluir o útil” em X min |
| M3 | **Orçamento do cache de intervalos** adaptativo (mais entradas se RSS ok; LRU por hit) | Mitiga H3 |
| M4 | **Evitar rematerializar** `Tabela[Coluna]` idêntica por linha quando o binding é invariante *em relação à linha atual* (já há `val_cache`; auditar misses pós-fix LET) | Menos alloc |
| M5 | **Corpus sintético 100k / 400k / 800k** com `LET`+`MATCH`+tabela (sem binário grande) + `xtask perf` de calc | Regressão objetiva **sem** depender do xlsx do usuário |

### 5.3 Estrutural (maior)

| ID | Ação | Notas |
|---|---|---|
| E1 | **Paralelismo por nível topológico** (`rayon` / `thread::scope` em chaves independentes do mesmo nível) | Exige `PassHost` sem `&mut` compartilhado; caches por thread ou shard; spills/dynamic arrays **serializam** |
| E2 | **Índice persistente entre passes** (não só por passada) para folhas estáticas | Invalidação por dirty range |
| E3 | **Compilação / specialize** de fórmulas de coluna idênticas (uma AST, muitos contextos ThisRow) | Grande ganho em tabelas **qualquer** workbook |
| E4 | **Wasm:** fatias cooperativas por frame (já previsto no plano de responsividade) | Sem threads reais |

Ordem sugerida: ~~Q1 → Q2~~ → **(Q4 ou Q3 conforme perfil)** → M2 → E1.
(Q1+Q2 fechados na 1ª sprint — ver §7.)

---

## 6. Critérios de aceite

Alvos para um corpus denso (~800k fórmulas, release, máquina de desenvolvimento típica);
o arquivo do usuário é um dos corpos de teste, não o único:

| Critério | Meta |
|---|---|
| UI durante calc | ≥ **30 fps** estável (ideal 60); sem “Não está respondendo” |
| Cancel | Interrompe e limpa status em **< 1 s** após o clique |
| Progresso | `done` reportado pelo menos a cada **200 ms** (mesmo se a fórmula atual for longa) |
| Conclusão full (F9) | Termina **ou** documenta teto; meta aspiracional **< 3 min** no corpus denso após quick wins |
| Automatic / edição | Viewport + cascata dirty úteis em **< 30 s**; resto em idle/F9 |
| Paridade | Mesmos valores que o caminho síncrono (testes de equivalência existentes + novos para índices) |
| Universalidade | Nenhum caminho de otimização gated por nome de aba/arquivo; fixtures sintéticas cobrem o padrão `LET`+`MATCH` |
| Regressão | `cargo xtask ci` verde (exceto wasm/clang de ambiente, como hoje) |

Não é aceite: “parece mais rápido” sem número; “UI ok” com job que nunca termina;
“só este xlsx ficou rápido”.

---

## 7. 1ª sprint — instrumentação + yield ✅ (2026-10-23)

**Objetivo:** provar se o platô em ~42% é **cauda de fórmulas** (H1/H2) ou **throughput uniforme** (só falta paralelismo/índice).

### Entregue no código

| Item | Onde / como |
|---|---|
| **Q1** Yield/report por tempo ~75 ms | `PassHost::coop` + `run_scoped` (nativo); wasm permanece por passos (256). API de teste: `CalcHooks::set_coop_interval_ms` / `set_coop_eager` / `set_slow_compute_us`. |
| **Q2** Stats off por padrão | `GRIDCRAFT_CALC_STATS=1` → stderr: p50/p95/p99/max, top-20 chaves, hit/miss `range_array` + equality, gap máx. entre reports, amostra de funções de topo. |
| Testes | `hooks_eager_coop_*`, `hooks_progress_reports_under_slow_compute`, `hooks_cancel_during_slow_compute` (`calc/src/tests.rs`). |

### Como ligar (manual / release)

```text
GRIDCRAFT_CALC_STATS=1  gridcraft.exe   # ou o binário release; F9 no corpus denso
# opcional (só debug/teste): GRIDCRAFT_CALC_SLOW_US=50000
```

Procurar linhas `[GRIDCRAFT_CALC_STATS]` no stderr ao fim de cada passada.

### 7.1 Sprint structured refs / dirty subset ✅ (2026-10-23)

**Problema:** `precedents` fazia `Expr::Struct(_) => dynamic = true`. Qualquer edit
incluía **todas** as fórmulas com structured ref via `graph.dynamic` (ex.: clear de ~13
células na coluna Referencia → dirty ~600k). A branch de jobs só adiava a UI; o custo
permanecia.

**Fix (universal):**
- `resolve_struct` em `calc/src/eval.rs` — mesma geometria que `Evaluator::structured`
  (ThisRow → linha da fórmula; coluna nomeada → corpo de dados; Headers/Totals/All;
  tabela implícita pelo `at`).
- `precedents(wb, sheet, at, expr)` passa a célula âncora; call sites em `rebuild` /
  `update_nodes`.
- Progresso em `run_scoped`: incremental usa `|dirty|` (cresce se spill reabrir trabalho);
  full mantém `graph.len()`.

**Medição (teste sintético):** 80 fórmulas ThisRow + clear de 13 células →
`cascade_exceeds(..., 40) == false` e progress total ≤ 40 (antes: dirty ≈ livro inteiro).

**Testes:** `structured_ref_precedents_are_static_when_resolvable`,
`table_column_edit_does_not_dirty_all_structured_formulas`,
`whole_column_structured_ref_dirties_column_readers_only`,
`nested_let_with_this_row_structured_refs`.

### 7.2 Follow-up: expand de tabela + grafo stale ✅ (2026-10-23)

**Problema:** com Areas de `Expr::Struct` pinadas no rebuild, auto-expand/paste que
cresce a tabela marcava `structural` e chamava `recalc_all` **sem** invalidar o grafo.
`ensure_graph` (otimização F9) via o grafo ainda “válido” e **não** reconstruía nós —
fórmulas de coluna calculada autofilled ficavam com valor Empty; precedents de coluna
inteira continuavam no range antigo.

**Fix (universal):** `invalidate_graph()` no branch estrutural síncrono de `commit`
(`engine/src/cmd/mod.rs`), simétrico ao path `defer_structural`.

**Testes:** `paste_expands_structured_calc_columns_like_cotacao` (engine),
`expanding_table_refreshes_whole_column_structured_precedents` (calc).

### 7.4 Índice de igualdade entre passes + ColEq em `=` (2026-10-26)

**Problema:** após F9 quente, editar escalares globais (ex. filial/UF) reavalia ~520k
fórmulas e rematerializa colunas de lookup estáticas (`col = codigo`, `MATCH(TRUE,…)`).

**Fix (universal, `calc`):**
- `Calc::persisted_eq` / `persisted_lookup_col`: reutiliza índices da passada anterior
  quando **nenhuma célula do intervalo** entrou no `dirty` da passada atual; limpa em
  `invalidate_graph`.
- `Evaluator::binary`: `col = escalar` → `Ev::ColEq` fora de `LET` (Q4 / `achou`).

**Testes:** `column_equality_in_expression_not_only_let_binding` (calc); regressão
`lista_v3_b1_b2_cascade_size` (engine, corpus V3).

**Pendente:** M2 com write-back parcial (só ordenação por aba ativa no `run_scoped`);
E1/E3; orçamento adaptativo M3.

### 7.5 Deduplicação do índice de igualdade por coluna (2026-10-26)

**Problema:** após §7.4, `equality hit` subiu no B1, mas `build` ainda ficava na ordem de
**~10⁵** por passada — chave `(sheet, RangeRef, norm)` diferia por âncora de linha
(`A100:A5000` vs `A2:A5000` na mesma coluna), reconstruindo o mesmo índice lógico.

**Fix (universal, `calc`):**
- Índice de coluna único por `(sheet, col, ColNorm)` com linhas **absolutas**; consultas em
  subintervalos filtram/reutilizam sem novo scan quando o span já está coberto.
- Linha única / plano 2D: mesma ideia para `(sheet, row, norm)` e `plane_eq` (COUNTIFS).

**Teste:** `column_equality_index_shared_across_row_subranges` (`calc`).

### Ainda pendente (decisão em dados — próxima sprint)

1. **Reteste manual** no corpus denso: clear/edit na coluna Referencia — dirty/progresso
   devem refletir só a cascata real (não ~600k por structured refs).
2. **Rodar** F9 em **release** com `GRIDCRAFT_CALC_STATS=1`.
3. **No platô ~40% (F9 full):** anotar se `done` congela > 5 s e o que o top-K / p99 mostram.
4. **Decisão:**
   - Poucas fórmulas dominam → **Q4** (first-true / lookup do padrão `achou`); Q1 já feito.
   - Custo uniforme + cache miss alto → **M3 + Q3**.
   - Uniforme, cache quente, 1 core → spike **E1**.

### 7.3 Pedido / PROCV + Tabela3 (2026-10-23)

**Sintoma:** `10 LISTA SC para Outros.xlsm`, aba **Pedido** — colar códigos em **A6+**
deixava **C–L** vazios nas linhas do **corpo** da Tabela3; linhas **novas** (expand)
preenchiam. Mudar estado (A3) ou Quant. (B) “acordava” o ecrã.

**Causa raiz (duas camadas):**

1. **Motor / seeds (corpus):** Tabela3 ≈ `A5:L13` (corpo 6–13). PROCV em C–L vem com
   cache Excel **`Text("")`** (`Value::is_empty()` falso). Expand cria linhas com
   **`Empty`** → calculam no job. Defer estrutural precisava de **`recalc_cascade`** com
   seeds do paste + fórmulas da linha (`touch_table_formula_rows_for_input_edits`,
   `needs_formula_eval`). Commits: **db8b630**, **55f3426**, **c73117e**.

2. **UI / paint (por que B2/B “funciona” e o paste “não”):** o motor **já** recalculava
   C6 após o job; a grelha guarda `text_cache` por `(uid, revision, sheet)`.
   - `Text("")` **entra** no cache (não é `Value::Empty` → não faz `continue` no paint).
   - Linhas novas `Empty` **não** são cacheadas até terem valor.
   - `apply_job` instalava o wb do worker **sem** `revision++` → cache hit com `""`
     nas linhas 6–13; C14+ pintavam fresco. Editar A3/B faz `push_undo` → `revision++`
     → cache invalidado → parece que “só A3/B disparam cascata”.

**Fix (universal) UI:** `ui-egui` `apply_job` (Calc Ok) → **`d.revision += 1`** após
aplicar o snapshot, para dropar text/CF caches. Complemento calc: `cells_changed`
sempre `rebuild` quando `!graph_valid` (também com grafo vazio pós-cancel).

**O que analisar (checklist):**
- [x] ListObject termina acima das linhas “presas” (corpo vs expand).
- [x] Cache **`Text("")`** vs `Empty` no xlsx e no paint.
- [x] Paste + `defer_structural` / `recalc_cascade` com seeds C6+.
- [x] Job aplica valores mas UI não pinta → **revision** / text_cache.

**Implementado:** §7.3 + `pedido_procv_repro.rs` + `pedido_paste_recalc.rs`
(`ui_calc_job_bumps_revision_so_text_cache_drops_stale_empty`).

**Validar:**
- `cargo test -p gridcraft-engine --test pedido_procv_repro`
- `cargo test -p gridcraft-ui-egui --test pedido_paste_recalc`
- Manual: abrir xlsm → Pedido → colar códigos em A6+ → esperar job → **C6** e **C14**
  com descrição **sem** precisar editar A3/B.

### Fora de escopo (mantido)

- Paralelismo completo de produção (E1 full).
- Reabrir truncate Automatic.
- Duplicar trabalho de responsividade já fechado.
- Qualquer otimização por nome de aba/workbook (§0).

---

## 8. Sugestão prioritária (resumo executivo)

1. **Tratar 42% como progresso real**, não hang — validar com Cancel < 1 s e 1 núcleo a 100%.
2. **Medir cauda** (histograma por fórmula) antes de paralelizar.
3. **Yield por tempo** para a barra não mentir em fórmulas monstro.
4. **Acelerar padrões de fórmula universais** (`LET` + comparação de coluna + `MATCH` / first-true) no motor — maior ROI provável no corpus denso; **não** special-case de aba.
5. **Full book só quando preciso**; viewport/dirty primeiro para “concluir o útil”.
6. **Paralelismo topológico** só depois de esgotar índices/cache (complexidade × spills).

---

## 9. Ligação com o plano de responsividade

| Tema | Onde vive |
|---|---|
| Never-freeze, JobManager, deferral, Fase 4/5 | [`plano-responsividade.md`](./plano-responsividade.md) |
| Throughput / conclusão / hipóteses do 42% | **Este arquivo** |
| Handoff operacional | Seção **ONDE PARAMOS** em `plano-responsividade.md` (aponta para cá) |

Ao fechar itens da §5, atualizar **ambos**: checklist aqui + uma linha no handoff de responsividade.
