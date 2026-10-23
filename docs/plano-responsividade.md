# Plano de responsividade — GridCraft não pode travar

> Objetivo: nenhuma ação do usuário bloqueia a thread da interface por mais de **~50 ms**
> (alvo: 16 ms por frame). Trabalho longo roda em **segundo plano**, com progresso e opção de
> cancelar. Alinha-se à regra "Never crash" do `CLAUDE.md`: travar = perder confiança.

Última análise: 2026-10-22. Caso de referência: `NOVA LISTA DE PRECO V19.xlsx`
(aba `LISTA_PRECO` com ~350 mil fórmulas, XML de ~192 MB, arquivo de ~5 MB).

> **Status da implementação (2026-10-23).** Fases **0, 1, 2 e 3** estão no código (Fase 3 completa
> no motor: jobs + índices + rebuild incremental do F9); **Fase 6 parcial** (perf + freeze guard);
> Fase 4 feita; **Fase 5 fechada** (Rodada 14–15: meta-diff + Arc no `CellStore` + undo Diff sem
> rebuild O(n); ver ONDE PARAMOS).
> **👉 Quem der continuidade: leia a seção "Handoff — próximos passos"** logo abaixo deste bloco.
> Resumo do que existe hoje:
>
> - `crates/ui-egui/src/jobs.rs`: `JobManager` (thread + canal `mpsc` + `AtomicBool` de
>   cancelamento; no wasm o trabalho roda no próprio frame, mesma API), `poll` por frame,
>   `cancel`, `is_saving`/`is_calculating`. Aplicação de resultados com guarda de documento
>   (uid), de snapshot (`Arc::ptr_eq`) e de formato.
> - `crates/engine/src/cmd/file.rs`: `SavePlan::prepare` (barato, thread da UI) +
>   `SavePlan::execute` (serializa + grava, sem `Session`). `file.save` roda os mesmos dois
>   passos inline, então há uma única implementação.
> - `crates/engine/src/lib.rs`: `recalc_document` (F9/Shift+F9 puro, com `CalcHooks` de
>   progresso/cancelamento e `CalcSeeds` para resultado idêntico ao síncrono).
> - AutoSave com **debounce de 5 s** após a última edição, um job por documento, nunca durante
>   edição nem sobre outro save; erros de autosave vão para o log (sem caixa de mensagem).
> - Barra de status com "Salvando… n %"/"Calculando… n %" e botão cancelar.
> - Fase 0: frame watchdog (> 100 ms) + medição por comando (> 50 ms), painel de depuração
>   **Ctrl+Alt+Shift+P** e o teste `every_command_stays_within_budget`.
> - Ainda inline (por decisão de escopo **na Rodada 1** — estado atual: ver as rodadas seguintes
>   e a seção Handoff): exportações em PDF e `saveBytes`, cascata de `cells_changed`, operações
>   estruturais, Solver/What-If/Cenários, aberturas por senha e `control`/MCP.
> - **Rodada 2 (2026-10-22).** "Salvar e fechar" implementado (o `file.close` enfileira e conclui
>   quando o job de save termina e o documento está limpo; um save que falha deixa o documento
>   aberto), exportações **CSV/HTML** viraram job, e o botão Paste dividido (WIP do ribbon) voltou
>   a ser clicável quando desabilitado — o que destravou os 4 testes de `clipboard.rs`
>   (`split_big_button` agora usa `Sense::click()` sempre e anuncia a seta com `widget_info`).
> - Testes novos: `crates/ui-egui/tests/background_save.rs` (fechar aguarda o job; save que falha
>   não fecha; exportação não mexe no documento).
> - `cargo xtask ci` nesta rodada: **fmt, clippy, test, assets e layers verdes**; o passo **wasm**
>   falha só por ambiente — falta `clang` para compilar o SQLite C (`libsqlite3-sys`) no
>   wasm32-unknown-unknown (`gridcraft-db`), o que independe destas mudanças.
> - **Rodada 3 (2026-10-22).** O núcleo da **Fase 3** está no código: comandos **estruturais** e
>   edições de **cascata enorme** em planilhas grandes não recalculam mais na thread da interface —
>   `commit` marca `recalc_pending` (`defer_structural`, ligado pela interface quando o grafo passa
>   de `job_thresholds.calc_formulas`; `Calc::cascade_exceeds` estima a cascata com saída antecipada
>   e `CASCADE_JOB_CELLS = 5 000` é o limite) e o job de cálculo reprocessa tudo, com cancelamento e
>   reenfileiramento a cada edição. Enquanto isso, a célula mostra o valor antigo e a barra de
>   status mostra "Calculando…". **`data.subtotal` reescrito em um único commit** (`Ctx`): no corpus
>   de 300 grupos caiu de **2 093 ms para ~480 ms**, e o undo reverte tudo em um passo (antes: um
>   commit por comando interno). Testes: `structural_commands_defer…` (engine) e
>   `a_structural_edit_recalculates_in_the_background` / `a_huge_cascade_recalculates_in_the_background`
>   (interface). Pendente na Fase 3 (estado atual): prioridade por viewport e aplicação parcial no
>   `run_scoped` — o restante (Solver/Atingir meta na Rodada 6; ajuste multi-inserção do subtotal
>   na Rodada 5) foi concluído.
> - **Rodada 4 (2026-10-22) — última antes deste handoff.**
>   - **Fase 6 (parcial):** `cargo xtask perf [--budget MS] [--rows N]` roda o teste de orçamento
>     com corpus escalável (`GRIDCRAFT_BUDGET_ROWS` no teste) e falha no primeiro comando acima do
>     limite; e o **freeze guard** dentro de `cargo xtask layers`: proíbe `save_bytes`,
>     `recalc_all`, `recalc_sheet`, `cells_changed`, `write_file` e `dbfile::save` em
>     `crates/ui-egui/src` (long work só via `jobs.rs`), e `load_document` só nos dois caminhos
>     assíncronos documentados em `lib.rs`. Negativamente testado. Nota: `std::fs::` de metadados
>     baratos (recentes, fontes, tema, screenshots) permanece permitido de propósito.
>   - **Fase 2 (compressão):** `SavePlan.compression` + `ZipOut::with_level` /
>     `write_xlsx_with` / `io::save_bytes_as_level` — **AutoSave grava com deflate nível 1**, o
>     salvar manual mantém 6.
>   - **Achado medido:** `data.subtotal` com 2 000 grupos (4 000 fórmulas) = **36,7 s** — o custo
>     restante é o `rewrite` de referências rodando **por inserção** (parse+print de _todas_ as
>     fórmulas a cada uma das 2 000 inserções). O redesign completo está na seção
>     **"Handoff — próximos passos"** abaixo, com o desenho pronto para implementar.
> - **Rodada 5 (2026-10-22) — Passo 2 do handoff (aberturas unificadas).**
>   - `SheetApp::open_document_async` é agora **o único loader** de abertura: `open_path_async`
>     (arquivo em disco) e `open_bytes_async` (inbox/web) são wrappers finos. O caminho
>     síncrono só permanece como fallback (wasm ou falha ao criar a thread).
>   - O **diálogo de senha** (OK) abre pelo loader assíncrono: planilha cifrada grande decifra
>     fora da thread da interface. Senha errada **não vira caixa de mensagem** — o
>     `LoadingTask` ganhou `retry` e reabre o diálogo com o campo limpo (o teste
>     `opening_an_encrypted_file_asks_for_its_password` fixa isso). O link de **recentes** também
>     abre assíncrono.
>   - O drain da abertura agora trata canal desconectado (thread morta) como erro, em vez de
>     deixar o modal de carregamento preso para sempre (mesmo princípio do `JobManager`).
>   - `control`/MCP (`app.open`) permanece **síncrono de propósito**: é o contrato de
>     requisição/resposta dos scripts (igual a CLI e testes).
>   - Nota de desempenho: o PBKDF2 do fixture cifrado usa `spin = 100 000`; em **debug** a
>     decifragem leva ~1–2 s por tentativa (em release, milissegundos). Os testes usam espera
>     limitada (`wait_for_open` / `wait_for_password_dialog`), nunca `run_steps` fixo.
> - **Rodada 6 (2026-10-22) — Passo 3 do handoff (Solver / Atingir meta como jobs).**
>   - `engine::run_analysis` roda `data.solver` / `data.goalSeek` numa **Session privada** sobre um
>     snapshot (mesma lógica do comando, zero acesso ao documento vivo) e devolve
>     `(workbook, calc, resultado JSON)`. `Session::apply_analysis` aplica isso ao documento real
>     como **um único passo de undo** (histórico + revisão + journal), sem recalcular.
>   - `CalcHooks` agora alimenta a iteração: o `Evaluator` do Solver reporta a cada 64 avaliações e
>     para no cancelamento; o Goal Seek reporta por iteração e para no cancelamento. **O cancelar
>     do F9 também passou a parar o cálculo em curso** (antes só descartava o resultado):
>     `Job::stop()` aciona os hooks, não só a flag.
>   - Interface: `JobKind::Analysis` + `JobResult::Analysis` (guarda por snapshot `Arc::ptr_eq`),
>     barra de status com "Solving…"/"Resolvendo…" (i18n nos 10 idiomas), e os diálogos ignoram a
>     resposta `pending` (o job abre o Solver Results / mostra o toast).
>   - Escopo de Cenários: `scenarioShow` é um commit barato (o deferimento de cascata já cobre
>     planilhas grandes); `scenarioSummary` só existe no catálogo (control/MCP), então permanece
>     síncrono pelo mesmo princípio de `app.open`.
>   - Testes: `analysis_runs_on_a_snapshot_and_honours_cancel` (engine; equivalência + snapshot
>     intocado + cancelamento de Goal Seek e Solver) e, na interface,
>     `goal_seek_runs_as_a_job_and_undoes_in_one_step` + `solver_runs_as_a_job_and_opens_the_results`.
> - **Rodada 7 (2026-10-22) — Passos 4 e 5 do handoff (Fase 4, partes paralelas no XLSX e priorização por viewport).**
>   - Escrita de partes (abas) grandes em paralelo no XLSX via `ZipOut::add_parts` com `thread::scope` e `merge_archive`.
>   - Cache de texto formatado por célula visível (`GridState.text_cache`), invalidado por `(uid, revision, sheet)`.
>   - Teto seguro de `MAX_FIT_ROWS = 1_000` em `cmd/mod.rs` no commit para auto-fitting de linhas.
>   - Priorização da aba ativa/scoped na fila dirty de `run_scoped` no recálculo (`recalc.rs`).
>   - Seção "Never freeze the UI" documentada em `docs/architecture.md`.
> - **Rodada 8 (2026-10-22) — Fase 3, otimizações do motor (cache de `range_values` por passada).**
>   - `Host::range_array` (novo) + `PassHost` memoriza, **por passada**, os intervalos
>     materializados como `Value::Array` (já são `Arc`): um acerto é clonar um ponteiro, em vez de
>     varrer/realocar o intervalo para cada fórmula. Orçamento de memória embutido
>     (`RANGE_CACHE_CELLS = 1,5 M` células, 128 intervalos); a chave inclui a **aba**.
>   - Correção primeiro: o cache fica **desligado** em planilhas com spill/dynamic arrays (um
>     spill pode preencher células no meio da passada); a suíte de spills continua verde.
>   - **Medido** (500 `SUMIF` sobre 20 mil linhas, mesmo intervalo): **3 175 ms → 188 ms (16,9×)**
>     antes/depois do cache. Teste determinístico: `range_cache_keeps_ranges_apart`.
>   - **Ainda na Fase 3 (motor) na época:** índice `COUNTIFS/SUMIFS`, índice `MATCH`/`VLOOKUP` e
>     `rebuild` incremental do grafo no F9. (Fechados: Rodadas 10–11 + medição *IFS; Rodada 12
>     rebuild incremental.)
> - **Rodada 11 (2026-10-22) — Fase 3: índice de igualdade para `COUNTIFS`/`SUMIFS`.**
>   - `gridcraft_functions::equality_operand` (criteria.rs) reconhece critério de **igualdade
>     puro** (`"x"`, `"=x"`, número, booleano; sem curingas, comparações, vazio ou erro) e devolve
>     o operando já coagido (texto numérico vira número, como o `Criterion`).
>   - `Host::equality_positions` (novo) + `PassHost::equality_positions` (recalc.rs): **índice por
>     intervalo por passada** (`criteria: HashMap<(aba, RangeRef), Key → Arc<Vec<u32>>>`), com o
>     mesmo orçamento de `RANGE_CACHE_CELLS`, chave numérica de 15 dígitos e desligado em planilhas
>     com spill (correção primeiro).
>   - `Ev::ifs_indexed` (eval.rs): atende `COUNTIFS`/`SUMIFS` **somente** com referências simples,
>     todas do mesmo formato do alvo (a regra da própria função), critérios de igualdade e
>     contagens/pares válidos; interseção das posições por dois ponteiros e soma só das células
>     casadas (erro na soma propaga como a função). Qualquer outro formato cai no caminho antigo.
>   - **Teste de equivalência:** `equality_indexed_ifs_matches_the_function_path` cobre
>     caixa, interseção de 2 critérios, célula como critério, texto na faixa de soma ignorado,
>     `#VALUE!` por formatos diferentes, curinga/comparação (fallback), 15 dígitos e compara
>     **caminho rápido × caminho da função** (forçado com `A2:A6&""`) nos mesmos casos.
>   - **Medido (2026-10-23):** cascata `cell.set` em `B1` com 10 mil `SUMIFS` sobre `A1:A50000`
>     (chaves únicas) — **7 083 ms → 94,4 ms (75×)** sem/com índice; baseline `=B1` × 10 mil na
>     mesma cascata **10,4 ms** (~1,0 µs/fórmula). Verificação: `equality_indexed_ifs…` verde.
> - **Rodada 9 (2026-10-22) — Fase 6 verificada e endurecida.**
>   - `cargo xtask perf --rows 2000 --budget 2000`: 335 comandos, **1 013 ms no total**, pior
>     comando 180 ms (`table.removeDuplicates`) — dentro do orçamento; a linha final confirma.
>   - O **freeze guard** (`cargo xtask layers`, parte do `xtask ci`) agora também varre o **shell
>     desktop** (`apps/gridcraft/src`), não só `crates/ui-egui/src`; o CLI segue fora do scan por
>     ser cliente batch do engine (como o próprio engine). `docs/architecture.md` atualizado.
>   - Verificado que `cmd_ci` inclui `("layers", cmd_layers)` — a guarda roda em todo CI.
> - **Rodada 10 (2026-10-22) — Fase 3, índice de `MATCH`/`VLOOKUP`: um bug real de igualdade.**
>   - **Descoberta:** o índice exato já estava **ligado** no despachante do avaliador
>     (`call`): `VLOOKUP`/`HLOOKUP` com `lookup_find_row`/`lookup_find_col` e `MATCH` exato
>     (linha e coluna). A tentativa de "ligar o índice" era redundante e foi removida.
>   - **Bug corrigido:** `to_lookup_exact_key` indexava números pelos **bits crus** do `f64`,
>     enquanto `lookup_equal` compara com a semântica de **15 dígitos** do Excel. Resultado: o
>     índice podia **não achar** valores iguais na 15ª casa (ex.: `MATCH(0.1+0.2, {0.3}, 0)` →
>     `#N/A` pelo índice, mas `1` pela varredura). Adicionados `core::round15` (público) e
>     `core::exact_number_key`, e o índice passou a usar a chave arredondada.
>   - **Teste de regressão:** `exact_lookup_index_matches_excel_semantics` cobre caixa
>     (case-insensitive), curingas, primeiro duplicado, referência de coluna inteira (trim),
>     limites/#N/A do `VLOOKUP`, a igualdade de 15 dígitos e uma forma que cai no fallback.
>     Provado: com a chave antiga (`to_bits`) o teste **falha** em `0.1+0.2 == 0.3`; com a
>     correção passa.
>   - **Fase 3 (motor) fechada na Rodada 12:** `rebuild` incremental do grafo no F9.
> - **Estado da árvore neste handoff:** `cargo fmt --all -- --check` limpo,
>   `cargo clippy --workspace --all-targets -- -D warnings` limpo, `cargo test --workspace`
>   sem falhas (305 no lib do engine, 7/7 em `background_save.rs`, clipboard 17/17), `assets` e
>   `layers` verdes; passo `wasm` do CI falha só por ambiente (falta `clang` na máquina para o
>   SQLite C — ver acima). Arquivos do WIP do dono intocados: `ribbon.rs` (split-button Paste,
>   com a correção de clique descrita na Rodada 2), `calc/src/eval.rs`, `cmd/edit.rs`,
>   `dbfile.rs`, `grid.rs`, `dialogs.rs`, `formula_locale.rs`, `.gitignore`.

---

## Handoff — próximos passos (para quem continuar o plano)

> ### ⏸️ ONDE PARAMOS — 2026-10-23 (Rodada fix recalc: cascata Automatic cross-sheet)
>
> **Regressão fechada.** Em Automatic, `Calc::cells_changed` deixou de truncar a dirty set à
> aba ativa quando `n > 1_000` (introduzido em `c7a3c16` para responsividade). Esse truncate
> deixava outras abas stale e fórmulas da aba ativa que leem cross-sheet “estáticas”; a troca
> de aba (`wb_switch_sheet` → `recalc_sheet_scoped` só em `value` vazio) mascarava o bug.
>
> **Correto agora:** cascata completa no caminho Automatic de edição. Cascata grande continua
> pelo deferral já existente (`defer_structural` + `CASCADE_JOB_CELLS` → `recalc_pending` / job
> F9), sem stale silencioso. Manual intacto. `recalc_on_load` pode seguir scoped (dívida de
> abertura / pending) — residual consciente.
>
> - Código: `crates/calc/src/recalc.rs` (`cells_changed`).
> - Testes: `automatic_cascade_keeps_cross_sheet_dependents_past_thousand`,
>   `manual_cells_changed_does_not_cascade_cross_sheet_dependents` (calc);
>   `automatic_edit_cascades_cross_sheet_or_marks_pending` (engine).
>
> **Herdado (Rodada 15 — ainda válida):** undo Diff sem rebuild O(n); Fase 5 COW; Fase 3 motor
> verde. Wasm/clang: não tentar.
>
> **PRÓXIMO PASSO RECOMENDADO (opcional):**
> 1. Índice `COUNTIF`/`SUMIF`/`AVERAGEIFS`/`MAXIFS`/`MINIFS` (extensão do índice *IFS da Fase 3).
> 2. Não commitir sem pedido do dono.


**Mapa do que já existe (símbolos):**
| O quê | Onde |
|---|---|
| JobManager, jobs de save/cálculo, cancelamento, guarda de aplicação | `crates/ui-egui/src/jobs.rs` (`JobManager`, `JobKind`, `JobResult`, `Thresholds`, `CANCELLED`) |
| Intercepts de comando → job (save/saveAs/exportCsv/exportHtml/F9, `file.close` diferido) | `crates/ui-egui/src/lib.rs` (`try_job_command`, `save_job_attempt`, `calc_job_attempt`, `close_job_deferral`, `finish_pending_close`, `export_job_attempt`, `apply_job`) |
| Orquestração Fase 3 (liga `defer_structural`, cancela/re-enfileira cálculo pendente) | `crates/ui-egui/src/lib.rs` em `after_engine` (bloco `#[cfg(not(wasm32))]`) |
| Save puro + atômico + nível de compressão | `crates/engine/src/cmd/file.rs` (`SavePlan::prepare/execute`, `SavePlan.compression`), `crates/engine/src/io.rs` (`save_bytes_as_level`, `write_file` atômico), `crates/xlsx/src/package.rs` (`ZipOut::with_level`), `crates/xlsx/src/{write.rs,lib.rs}` (`write_xlsx_with`) |
| Recálculo deferido (Fase 3) | `crates/engine/src/cmd/mod.rs` `commit` (`defer_structural`/`recalc_pending`, `CASCADE_JOB_CELLS`), `crates/engine/src/lib.rs` (`DocState.defer_structural/recalc_pending`, `recalc_document`, `doc_by_uid_mut`), `crates/calc/src/recalc.rs` (`CalcHooks`, `CalcSeeds`, `Calc::cascade_exceeds`) |
| Subtotal em um commit (falta o ajuste multi-inserção) | `crates/engine/src/cmd/data.rs` (`subtotal`, `insert_row_on`, `set_text/set_formula`, `bold_range`, `group_rows_on`) |
| Medição (Fase 0) | `crates/engine/src/tests.rs` `every_command_stays_within_budget` (env `GRIDCRAFT_BUDGET_MS`, `GRIDCRAFT_BUDGET_ROWS`), painel Ctrl+Alt+Shift+P (`widgets::perf_panel`) |
| Guarda-corpos (Fase 6) | `xtask/src/main.rs` (`cmd_perf`, `freeze_guard` dentro de `cmd_layers`) |
| Testes de comportamento | `crates/ui-egui/tests/background_save.rs`, `crates/calc/src/tests.rs` (hooks), `crates/engine/src/cmd/data.rs` (subtotal funcional) |
| Fase 5 — bandas COW + undo por diff | `crates/model/src/store.rs` (`Arc` interno + `BAND=64`, `content_diff`), `crates/model/src/patch.rs` (`WorkbookDiff` / `SheetMeta`), `crates/engine/src/lib.rs` (`HistoryState::Diff`, `push_undo`, `apply_history_state`), `cmd/edit.rs` (`ensure_graph` no undo/redo — R15) |

**Próximo passo 1 — ✅ CONCLUÍDO (Rodada 5, 2026-10-22): subtotal com ajuste multi-inserção.**
Medido: 2 000 grupos × 4 000 fórmulas **36,7 s → ~60 ms**; `cargo xtask perf --rows 2000 --budget 2000` passa.
O que foi feito (o desenho abaixo foi seguido, com um acréscimo): `formula::adjust::adjust_edits`
(atalho sem alocação para listas só de inserção de linhas), `rewrite_all_formulas_edits` /
`rewrite_edits`, e `subtotal` reordenado (deslocamentos → UM rewrite → escrita em coordenadas
finais). **Achado:** só o rewrite em lote dava ~3,6 s; o gargalo restante era `CellStore::shift_rows`
(varre e reinsere todas as células por inserção) — resolvido com `CellStore::remap_rows`, que move
todas as células numa passada (`crates/model/src/store.rs`). Texto original do desenho:
Medido: 2 000 grupos × 4 000 fórmulas = **36,7 s**; a causa é `cmd/sheet.rs::rewrite` rodando
**por inserção** (parse+print de todas as fórmulas do workbook a cada uma das k inserções —
O(fórmulas × k)). Desenho pronto:

1. `crates/formula/src/adjust.rs`: `pub fn adjust_edits(e: Expr, host: &str, target: &str, edits: &[Edit]) -> Expr`
   = fold de `adjust` na ordem cronológica (em memória, sem reparsar). (Opcional, se ainda pesar:
   forma fechada para "só inserções de 1 linha" — cada âncora de linha shift por
   `upper_bound(inserts_ordenados, row)`, O(log k) por referência.)
2. `crates/engine/src/cmd/edit.rs`: trocar `rewrite_all_formulas` por um
   `rewrite_all_formulas_edits(wb, target, edits: &[Edit])` — mesmo walk, `adjust_edits` por
   fórmula, `moves_arrays` com `.any()`, e para arrays legados
   `edits.iter().fold(Some(a), |r, e| r.and_then(|r| super::sheet::map_range(r, e)))`.
3. `crates/engine/src/cmd/sheet.rs`: `rewrite_edits(wb, target, edits)` (mesmo walk de
   charts/sparklines com o `fix` usando `adjust_edits`); `rewrite` (single) delega.
4. `crates/engine/src/cmd/data.rs` `subtotal`: reordenar para **(a)** shifts de todas as linhas
   (desc, sem `rewrite` entre eles — `insert_row_on` ganha um parâmetro/variante sem rewrite),
   **(b)** UM `rewrite_edits` com todos os `Edit::Insert` em ordem cronológica, **(c)** escrever
   rótulos/fórmulas/bold/outline **em coordenadas finais** (grupo i em ordem crescente:
   dados `g0+i..=g1+i`, linha de total `g1_i+1+i`, grand total em `r.end.row+n+1`, faixa do grand
   `r.start.row+1 ..= last-1` 0-based — o teste existente fixa B2:B3/B5:B6/B2:B7). **Importante:**
   o `rewrite_edits` tem de vir ANTES da escrita das novas fórmulas (elas já nascem nas
   coordenadas finais e não podem ser ajustadas de novo).
   Esperado: 36,7 s → ~1–3 s em 2 000 grupos; `cargo xtask perf --rows 2000 --budget 2000` passa.

**Próximo passo 2 — ✅ CONCLUÍDO (Rodada 5, 2026-10-22): aberturas unificadas.**
`SheetApp::open_document_async` é o loader único (parâmetros de `file.open`: `path` ou
`name`+`base64`, opcionalmente `password`); `open_path_async`/`open_bytes_async` são wrappers
finos e o caminho síncrono fica só como fallback (wasm/falha de thread). O diálogo de senha
(OK) e o link de recentes abrem em thread; `LoadingTask.retry` faz uma senha errada reabrir o
diálogo com o campo limpo (sem caixa de mensagem). O drain trata canal desconectado como erro
(thread morta nunca deixa o modal preso). `control`/MCP permanece síncrono de propósito.
Testes: `tests/open_encrypted.rs` (6 casos, com `wait_for_open`/`wait_for_password_dialog`).
Escrita das partes (abas) grandes em paralelo no xlsx concluída (`ZipOut::add_parts`).

**Próximo passo 3 — ✅ CONCLUÍDO (Rodada 6, 2026-10-22): Solver / Atingir meta como jobs.**
`engine::run_analysis` + `Session::apply_analysis` (um undo), hooks de progresso/cancelamento no
`Evaluator` do Solver e no laço do Goal Seek, `JobKind::Analysis` na interface e barra "Solving…".
Cenários: `scenarioShow` já é coberto pelo deferimento de cascata; `scenarioSummary` fica síncrono
(só catálogo/control). **Resta na Fase 3:** prioridade por viewport e aplicação parcial no
`run_scoped` (otimização de um caminho já assíncrono).

**Próximo passo 4 — ✅ CONCLUÍDO (2026-10-22): padrão "never-freeze" documentado.**
Seção "Never freeze the UI" em `docs/architecture.md` (jobs, SavePlan, `defer_structural` /
`recalc_pending`, guardas de aplicação, freeze guard, `xtask perf`). `std::fs` de metadados
baratos permitido de propósito está registrado ali e no comentário do `freeze_guard`.
(Resta apenas contribuir o padrão ao `craftrules`, se o dono quiser.)

**Próximo passo 5 — ✅ CONCLUÍDO (Rodada 7, 2026-10-22): Fase 4 (orçamento por frame) e priorização por viewport.**

- Cache de string formatada por célula visível em `grid.rs` (`GridState.text_cache`, invalidado por `(uid, revision, sheet)`).
- `auto_row_height` limitado com teto seguro (`MAX_FIT_ROWS = 1_000`) no commit para evitar congelamento de UI em edições massivas.
- `run_scoped` no recálculo ordena e prioriza as células da aba ativa/visível em cada rodada.
- Escrita de partes (abas) grandes em paralelo no XLSX (`ZipOut::add_parts`).

**Invariantes que os testes fixam (não quebrar):**

- CLI, testes de engine e o canal de controle nunca ligam `defer_structural` — contrato síncrono
  intacto lá; só `SheetApp::after_engine` liga (nativo; wasm nunca difere).
- Um job de save por documento; `saved` = snapshot efetivamente gravado; gravação atômica.
- Resultado de cálculo só aplica com `Arc::ptr_eq` do snapshot; edição durante cálculo cancela e
  reenfileira; `formulas.calculateNow` inline quita `recalc_pending`.
- Undo do subtotal é um passo único (o teste funcional fixa o layout exato).

**Como verificar a árvore:**

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo xtask layers      # inclui o freeze guard
cargo xtask perf --rows 2000 --budget 2000   # passa desde a Rodada 5 (subtotal ~60 ms)
```

---

## Find & Replace (Ctrl+L) — especificação do módulo

**Estado.** O diálogo existe (`crates/ui-egui/src/dialogs.rs::find`) e o motor tem
`edit.find` / `edit.replace` (curingas `*`/`?` com escape `~`, caixa, célula inteira,
`lookIn: formulas|values`, `within: sheet|workbook`, `byColumns`; **Substituir tudo é um único
undo**). O que **não** funcionava era o atalho:

> **Causa raiz do Ctrl+L:** `crates/ui-egui/src/keymap.rs` tinha `L if cmd => run("insert.table")`
> — o `keymap::dispatch` roda antes de `ribbon::shortcut` e consumia a tecla, então a busca nunca
> abria (e o atalho era um duplicado de Ctrl+T, que já insere tabela). **Corrigido** para
> `L if cmd => Shortcut::Dialog("find", {})` no Windows/Linux. No macOS ficam intactos
> Cmd+L = Alinhar à Esquerda e Ctrl+L = Definir Nome (Excel for Mac). O teste de unidade do
> keymap foi atualizado (`tests/keyboard_shortcuts.rs`) e há teste de UI novo
> (`tests/find_replace.rs`).

**Atalhos (contrato):**

| Tecla | Ação |
|---|---|
| **Ctrl+L** | Localizar (aba Localizar) — convenção pt-BR, Windows/Linux |
| Ctrl+F | idem (convenção EN; já existia) |
| Ctrl+H | Localizar e Substituir (aba Substituir) |
| Shift+F5 / F5 | Localizar Próximo |
| Enter (no diálogo) | Localizar Próximo |
| Esc | Fecha |

**O que o módulo deve ter (checklist):**

- [x] Abrir por Ctrl+L / Ctrl+F (Localizar) e Ctrl+H (Substituir).
- [x] Pré-preencher com a última busca (`Session::last_find`).
- [x] Opções: coincidir caixa; célula inteira; **Look in: Fórmulas/Valores**;
      **Search: por linhas/colunas**; **Within: planilha/pasta de trabalho**.
- [x] Curingas `*`/`?` com escape `~`.
- [x] **Localizar tudo** → lista com Planilha, Célula, Valor e **Fórmula** (novo); clicar
      navega (ativa a aba, seleciona e rola até a célula).
- [x] Substituir / Substituir tudo → contagem exibida e **um único passo de undo**.
- [ ] **Pasta (workbook) como coluna e busca entre documentos abertos.** Hoje `edit.find` busca
      só no documento ativo; a coluna "Pasta" do diálogo de referência exige iterar
      `Session::documents()` e devolver `{doc, sheet, cell, value, formula}`; clicar ativa o
      documento (`window.activate`) antes de selecionar a célula.
- [ ] **Nome** (nome definido) como coluna: casar também `wb.names` e mostrar o Nome.
- [ ] **Orçamento para planilhas grandes:** Localizar tudo varre todas as células na thread da UI
      (O(células) por comparação). Medir no corpus do `xtask perf`; acima do orçamento, virar job
      do `JobManager` (progresso + cancelar), como F9/save.
- [ ] "Localizar em" iniciando pela seleção (planilha × pasta de trabalho), como no Excel.
- [ ] Combobox com histórico de buscas (o diálogo de referência tem dropdowns).
- [ ] Acessibilidade: rótulos estáveis nos botões/campos (os testes kittest os usam).

**Como garantir o funcionamento completo (testes/aceite):**

- **UI** (`crates/ui-egui/tests/find_replace.rs`): Ctrl+L abre na aba Localizar; Ctrl+H na aba
  Substituir; Localizar tudo devolve célula **e fórmula**; clique no resultado navega.
- **Motor** (`cmd/edit.rs`): `matches` (caixa, célula inteira, `*`/`?`, `~`), `lookIn`
  fórmulas × valores, `within` planilha × pasta, ordem por linhas × colunas, Substituir tudo em
  um undo, e "não encontrado".
- **Orçamento:** incluir `edit.find`/`edit.replace` no relatório de
  `every_command_stays_within_budget` (limite próprio, sem falhar por enquanto).
- **Aceite manual:** numa planilha de ~350 mil fórmulas, Ctrl+L abre em < 100 ms; Localizar tudo
  de um termo comum responde com progresso (ou < 1 s); Substituir tudo desfaz em um passo.

---

## 1. Diagnóstico

### 1.1 O que já existe (reaproveitar)

| Item                              | Onde                                                                                       | Observação                                                                              |
| --------------------------------- | ------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------- |
| Abertura em thread                | [lib.rs `open_path_async`](../crates/ui-egui/src/lib.rs) + `LoadingTask`/`LoadTaskMessage` | Padrão a copiar: thread + canal `mpsc` + modal de progresso (`widgets::loading_modal`). |
| `load_document` sem `Session`     | [cmd/file.rs](../crates/engine/src/cmd/file.rs)                                            | Prova que o engine permite trabalho fora da `Session`.                                  |
| `Workbook` imutável compartilhado | `DocState.wb: Arc<Workbook>`, folhas em `Arc<Sheet>`                                       | Dá um **snapshot barato** para jobs em segundo plano.                                   |
| Scan de recentes / fontes / tema  | `backstage.rs`, `fonts.rs`, `desktop_theme.rs`                                             | Já em threads.                                                                          |

### 1.2 Causa-raiz

Toda a lógica roda **dentro do `update()` do egui**: um clique chama `SheetApp::run` →
`Session::execute` → comando → (cálculo + serialização + disco), tudo síncrono. Qualquer
operação O(n) com n grande congela a janela, e o Windows marca "Não está respondendo".

### 1.3 Inventário de pontos que bloqueiam (encontrados na análise do código)

Severidade: 🔴 trava por segundos+ em planilhas grandes · 🟠 trava em casos grandes · 🟡 risco menor.

| #   | Ponto                                                                                                            | Local                                                                                       | Sev.  | Por que trava                                                                                                                            | Tratamento (seção 3)                                      |
| --- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- | ----- | ---------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| 1   | **Salvar** (`file.save`/`saveAs`)                                                                                | `engine/cmd/file.rs::save` → `io::save_bytes_as` + `io::write_file`                         | 🔴    | Serializa XML de todas as abas, comprime (deflate nível 6, `xlsx/package.rs`) e grava, na thread da UI.                                  | Job de salvamento (fase 2)                                |
| 2   | **AutoSave** a cada 2 s                                                                                          | `ui-egui/lib.rs` (~linha 1338, bloco "AutoSave")                                            | 🔴    | Chama `session.run("file.save")` **no frame**, repetidamente enquanto houver alteração. É o pior ofensor: congela de novo a cada edição. | Autosave em job, com debounce e sem sobreposição (fase 2) |
| 3   | **F9 / Calcular agora**                                                                                          | `engine/cmd/formulas.rs::calc_now` → `Calc::recalc_all`                                     | 🔴    | `rebuild` do grafo + avaliação de todas as fórmulas. (Parcialmente mitigado: `MATCH` indexado.)                                          | Job de cálculo (fase 3)                                   |
| 4   | **Editar célula com dependentes**                                                                                | `engine/cmd/mod.rs::commit` → `Calc::cells_changed`                                         | 🔴/🟠 | Recalcula toda a cascata de dependentes, síncrono. Uma célula de entrada (ex.: `COTACAO!B1`) invalida 350 mil fórmulas.                  | Cálculo incremental com orçamento de tempo (fase 3)       |
| 5   | **Clone do workbook a cada edição**                                                                              | `cmd/mod.rs::commit`: `(*d.wb).clone()`                                                     | 🟢    | `CellStore` com Arc interno + undo `WorkbookDiff` (células e meta de aba). Aceite < 20 ms sem cascata no proxy 50k.                     | Fase 5 fechada — Rodada 14                                |
| 6   | **Fechar com alterações / Salvar e fechar**                                                                      | `cmd/file.rs::close` + diálogo "saveChanges"                                                | 🔴    | Mesmo caminho do salvar.                                                                                                                 | Reusa job de salvar                                       |
| 7   | **Exportar** (PDF, CSV, XLSX, ODS)                                                                               | `file.export*`, `cmd/print.rs::export_pdf`, `backstage.rs::export_as`                       | 🔴    | Renderização/serialização síncrona; `export` ainda faz `(*d.wb).clone()`.                                                                | Jobs de exportação (fase 2)                               |
| 8   | **Abrir** em `file.open` direto                                                                                  | `cmd/file.rs::open`, `SheetApp::open_path` (fallback sync), senha, drag&drop, `control`/MCP | 🟠    | Só `open_path_async` é assíncrono; os demais caminhos chamam `load_document` na UI.                                                      | Unificar tudo em `open_path_async` (fase 2)               |
| 9   | **`recalc_on_load` (pós-abertura)**                                                                              | `DocState::new`                                                                             | 🟠    | Roda na thread do loader (ok), mas depois a aba ativa pode precisar de `recalc_sheet_scoped` na UI.                                      | Mover para o job de abertura                              |
| 10  | **Desfazer / Refazer**                                                                                           | `edit.undo/redo`                                                                            | 🟢    | Diff: `cells_changed`/`update_nodes` + `ensure_graph` (R15); Snapshot ainda rebuilda.                                                    | Fechado — Rodada 15                                       |
| 11  | **Operações estruturais** (inserir/excluir linhas e colunas, ordenar, filtrar, remover duplicatas, colar grande) | `commit` com `structural=true` → `recalc_all`                                               | 🔴    | Qualquer estrutural dispara recálculo total síncrono.                                                                                    | Job de cálculo + estado "recalculando…" (fase 3)          |
| 12  | **`auto_row_height`**                                                                                            | `commit` (`fit_rows`)                                                                       | 🟡    | Medição de texto para até 200 mil linhas.                                                                                                | Limitar/adiar para linhas visíveis                        |
| 13  | **Pintura da grade**                                                                                             | `ui-egui/grid.rs` (`is_dirty`, leitura de valores por frame)                                | 🟡    | Valores de fórmulas lidos a cada frame; `d.is_dirty()` e `display_title()` em todo frame (baratos hoje, manter assim).                   | Orçamento por frame (fase 4)                              |
| 14  | **Ferramentas pesadas** (Solver, Atingir meta, Cenários, tabelas de dados, consolidar, SUBTOTAL grande, pivôs)   | `engine/cmd/solver.rs`, `whatif.rs`, `analysis.rs`                                          | 🔴    | Iteram recalculando o modelo.                                                                                                            | Job genérico cancelável (fase 3)                          |
| 15  | **Conexões/DB/importações**                                                                                      | `dbfile.rs`, `file.importXlsxToDb`, `data.getData`, `fromTextCsv`                           | 🔴    | I/O e parsing síncronos.                                                                                                                 | Job genérico (fase 2)                                     |
| 16  | **Impressão / pré-visualização**                                                                                 | `print.rs`, `ui-egui` (native_print)                                                        | 🟠    | Paginação e render completos.                                                                                                            | Job + cache de páginas                                    |
| 17  | **Clipboard grande** (copiar/colar)                                                                              | `lib.rs::copy_to_clipboard`, `paste_*`                                                      | 🟠    | Serializa o intervalo em texto/HTML na UI.                                                                                               | Limite + job                                              |
| 18  | **Canal de controle / MCP**                                                                                      | `control.rs`, `crates/mcp`                                                                  | 🟡    | Executa comandos na UI thread (por desenho).                                                                                             | Aceitável; usar mesmo scheduler                           |
| 19  | **Diálogos nativos de arquivo**                                                                                  | `rfd` (`file_menu` tests)                                                                   | 🟡    | Bloqueiam enquanto abertos (comportamento normal do SO).                                                                                 | Já tratável em thread própria                             |

> A tabela é um **ponto de partida**. A seção 2 descreve como achar o que ainda não foi listado.

### 1.4 Caso F9 — o que já foi feito e o que ainda falta

**Já feito (mitigação, 2026-10-22):** `MATCH(x, intervalo, 0)` passou a usar o índice hash por
passada (o mesmo do `PROCV` exato) e deixou de clonar o vetor a cada chamada
([eval.rs](../crates/calc/src/eval.rs), [lookup.rs](../crates/functions/src/lookup.rs)).
No arquivo real, o cálculo das 351.015 fórmulas passou de **> 100 s sem terminar** para **~12 s**
(CLI, release).

**O que ainda falta (pendente):** o cálculo **continua rodando na thread da interface**
(`calc_now` → `Calc::recalc_all`). Com 350 mil fórmulas, o F9 ainda deixa a janela parada por
alguns segundos antes de atualizar, e o Windows pode mostrar "Não está respondendo".
A solução é mover o recálculo para **uma thread em segundo plano com barra de progresso** —
mudança maior, tratada na **Fase 3** (itens abaixo), que depende do `JobManager` da Fase 1:

- [x] `calc_now` vira job: cópia/snapshot do `Workbook` → `Calc` em thread → resultado aplicado em lote.
      _(Somente quando o grafo passa de `job_thresholds.calc_formulas`, hoje 30 mil fórmulas; abaixo
      disso o caminho inline continua, o que mantém testes e canal de controle síncronos.)_
- [x] Barra de progresso "Calculando… n %" (contagem de fórmulas avaliadas / total) e botão cancelar.
- [x] `Calc::run_scoped` reporta progresso e checa o cancelamento a cada N fórmulas.
      _(progresso a cada 4096 fórmulas; cancelamento a cada fórmula.)_
- [x] Windows não marca "Não está respondendo": a UI continua bombeando eventos e repintando.
- [x] F9 repetido ou edição durante o cálculo cancela o job anterior e reenfileira.
- [x] Reduzir os ~12 s restantes: `rebuild` incremental do grafo (Rodada 12), índice
      `COUNTIFS/SUMIFS` (Rodada 11), cache de `range_values` (Rodada 8). Paralelizar por aba/bloco
      fica para Fase 5 / otimizações futuras.
- **Aceite do F9:** com `NOVA LISTA DE PRECO V19.xlsx`, a janela segue fluida (≥ 30 fps) durante todo o
  cálculo, mostra o progresso e o resultado final é idêntico ao do cálculo síncrono.

---

## 2. Como identificar TODOS os pontos (método, não só lista)

Não dá para confiar em achar tudo "no olho". Usar quatro camadas, em ordem de custo:

### 2.1 Medir de verdade: _frame watchdog_ (instrumentação permanente)

Adicionar em `SheetApp::update` (já existe `perf.frame_ms`):

- Se `frame_ms > 100`, registrar `{id do comando, duração, tamanho do workbook}` num log
  (`GRIDCRAFT_PROFILE` ou arquivo `slow-frames.log`).
- Em `SheetApp::run_command` medir **cada comando** (`id`, ms). Comando > 50 ms entra no relatório.
- Painel de depuração opcional (Ctrl+Alt+Shift+P) com os 20 piores.
- Em CI/headless: teste que falha se qualquer comando exceder o orçamento no corpus de teste.

Isso transforma "o sistema travou" em **dado**: lista ordenada de comandos lentos.

### 2.2 Varredura estática (grep guiado)

Rodar, na raiz, e revisar cada ocorrência chamada a partir de `update()`:

```powershell
# I/O síncrono
rg -n "std::fs::|File::(open|create)|read_file|write_file|save_bytes|open_bytes" crates/ui-egui/src crates/engine/src
# Recálculo / reconstrução de grafo
rg -n "recalc_all|recalc_sheet|rebuild\(|cells_changed|calc_now" crates
# Clones caros do workbook
rg -n "\(\*d\.wb\)\.clone\(\)|wb\.clone\(\)|make_mut" crates/engine/src
# Laços sobre toda a planilha
rg -n "sheet\.cells\.iter\(\)|used_range\(\)|\.iter_range\(" crates/engine/src crates/ui-egui/src
# Sleep/espera bloqueante
rg -n "thread::sleep|recv\(\)|join\(\)|block_on" crates
```

Marcar cada resultado como: **(a)** já em thread, **(b)** barato, **(c)** precisa de job.
Guardar o resultado na tabela da seção 1.3.

### 2.3 Corpus de planilhas pesadas + teste de orçamento

- Criar fixtures **geradas em código** (regra do repo: sem binários grandes): 100 k / 350 k / 1 M
  células, com fórmulas `VLOOKUP/MATCH/SUMIFS/COUNTIFS`, tabelas estruturadas, várias abas.
- Teste de integração que executa cada comando do catálogo (`catalog.rs`) sobre esse corpus e
  mede o tempo. Estende o `every_command_survives_empty_params`: criar
  `every_command_stays_within_budget` (limites: 50 ms edição simples; "job" permitido para o resto).
- Rodar o caso real do usuário manualmente (sem commitar o arquivo).

### 2.4 Detecção de travamento em produção (opcional)

- Thread _watchdog_ que verifica um "batimento" por frame; se passar de 2 s sem batimento,
  grava a pilha/o último comando em `crash/hang-<data>.log` (ajuda a achar o que escapou).

---

## 3. Arquitetura proposta

### 3.1 Princípio

> A UI thread **só desenha e despacha**. Comandos "curtos" executam inline; comandos "longos"
> viram **Jobs**.

### 3.2 Componente: `JobManager` (novo, em `ui-egui`, camada L6)

```
SheetApp
 └─ jobs: JobManager
      ├─ spawn(kind, title, snapshot, work) -> JobId
      ├─ poll(ctx)              // chamado a cada frame: drena canal, aplica resultados
      ├─ cancel(JobId)          // AtomicBool de cancelamento
      └─ progress: Vec<JobView> // para a barra de status / modal
```

- Cada job: `std::thread` (nativo) com canal `mpsc` (`Progress(f32,String)`, `Done(Result)`),
  `Arc<AtomicBool>` para cancelar, `ctx.request_repaint()` ao progredir.
- **Wasm**: sem threads reais → executar em fatias (_cooperative chunks_) a cada frame, com
  orçamento de ~8 ms por fatia (mesma API, outro executor). Regra do repo: **nunca quebrar wasm**
  (`cargo xtask wasm`).
- Um único job de **escrita** por documento por vez (fila); leituras podem coexistir.

### 3.3 Modelo de dados: snapshot + resultado versionado

- Job recebe `Arc<Workbook>` (clone barato) + `revision` do documento.
- Ao concluir, o resultado só é **aplicado** se `doc.revision` ainda for o mesmo; senão:
  - salvar: grava o snapshot mesmo assim e marca o doc como "alterado desde o salvamento"
    (`saved` = snapshot salvo, não o `wb` atual → `is_dirty` continua verdadeiro);
  - cálculo: descarta ou reaplica sobre o `wb` atual (ver fase 3).
- Nada de `&mut Session` dentro da thread. O engine expõe funções puras, como já faz `load_document`.

### 3.4 UX durante jobs

- Barra de status: "Salvando… 42 %" + botão cancelar; ribbon continua responsivo.
- Enquanto salva: edição **permitida** (graças ao snapshot). Fechar o app espera o job.
- Cálculo em andamento: indicador "Calculando…" na barra de status (como o Excel);
  células ainda não recalculadas mostram o valor antigo.
- Erros de job aparecem no `message` box padrão, sem derrubar o app (regra _never crash_).

---

## 4. Fases de implementação

Cada fase termina verde em `cargo xtask ci` (fmt, clippy `-D warnings`, testes, assets, layers, wasm)
e vira um commit por tarefa, conforme o `CLAUDE.md`.

### Fase 0 — Medir (1–2 dias) · pré-requisito

- [x] Frame watchdog + medição por comando (seção 2.1).
      _(`SheetApp::ui` registra frames > 100 ms com o tamanho da planilha; `run_command` mede cada
      comando e guarda os > 50 ms; painel de depuração em Ctrl+Alt+Shift+P.)_
- [x] Gerador de fixtures pesadas + teste `every_command_stays_within_budget` (2.3), inicialmente
      só **reportando** (sem falhar). _(Corpus gerado em código; o teste lista os 25 piores e só
      falha com `GRIDCRAFT_BUDGET_MS` definido. Ver os tempos medidos abaixo.)_
- [x] Preencher a tabela 1.3 com tempos reais (coluna "ms medidos").
- **Aceite:** relatório ordenado dos comandos mais lentos no corpus e no arquivo real.

**Medido no corpus gerado (18 000 células / 3 000 fórmulas, `cargo test` debug, 2026-10-22):**

| Comando                  |   ms | Observação                                                                                                                                                                                                                                                                                                                                 |
| ------------------------ | ---: | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `data.subtotal`          | ~480 | **Corrigido na Rodada 3:** era 55 710 ms (2 093 ms no corpus de 300 linhas) — ~quadrático porque cada um dos 4 comandos internos por grupo recalculava a cascata inteira. Reescrito em um único commit (`Ctx`): a medição acima é o novo custo, que resta O(grupos×fórmulas) no `rewrite` de referências (ajuste multi-inserção pendente). |
| `table.removeDuplicates` |  162 |                                                                                                                                                                                                                                                                                                                                            |
| `data.removeDuplicates`  |   91 |                                                                                                                                                                                                                                                                                                                                            |
| `home.deleteRows`        |   73 |                                                                                                                                                                                                                                                                                                                                            |
| `file.saveBytes`         |   64 | Serialização + compressão.                                                                                                                                                                                                                                                                                                                 |
| `home.deleteColumns`     |   57 |                                                                                                                                                                                                                                                                                                                                            |

Os demais ficam abaixo de ~35 ms. O corpus do teste foi reduzido para 300 linhas (~2,3 s no total)
para não pesar no `cargo test`; o número acima é a medição completa, pontual.

### Fase 1 — Infra de jobs (2–3 dias)

- [x] `JobManager` (nativo: thread+canal; wasm: fatias por frame).
      _(No wasm o trabalho ainda roda inteiro dentro do frame — mesma API de propósito, para um
      executor por fatias entrar sem mudar chamadores.)_
- [x] Barra de status + modal/overlay de progresso reutilizando `loading_modal`.
      _(Barra de status com progresso e cancelar; o `loading_modal` continua para aberturas.)_
- [x] Cancelamento cooperativo (`AtomicBool` checado em laços longos).
- [x] Testes: job conclui, job cancelado, job com erro, job com `revision` obsoleta.
- **Aceite:** um job de brinquedo (sleep 3 s) roda sem congelar a UI (teste com `egui_kittest`,
  mesmo estilo do teste existente "clicking Paste froze the UI thread").
  _(Coberto pelos testes unitários do `JobManager` (concluir/cancelar/erro/snapshot) e pela
  abertura em thread do `open_encrypted`; não há um teste kittest com `sleep` dedicado.)_

### Fase 2 — I/O em segundo plano (3–5 dias) · **resolve "Salvar"**

- [x] Extrair de `cmd/file.rs::save` uma função pura
      `prepare_save(&DocState) -> SavePlan` (barata, UI thread) e
      `execute_save(SavePlan, progress) -> Result<SaveOutcome>` (job).
      _(Nomes no código: `SavePlan::prepare` e `SavePlan::execute`; `file.save` roda os dois.)_
- [x] Gravação **atômica**: escrever em `arquivo.xlsx.tmp` e renomear (evita corromper se o app
      fechar no meio) — também protege contra o pior caso de salvar em thread.
      _(Já existia em `io::write_file`.)_
- [x] `d.saved` passa a ser o snapshot efetivamente gravado (corrige o `is_dirty` com edição durante o save).
- [x] **AutoSave** (lib.rs): debounce (ex.: 5 s sem edição), no máximo 1 job por documento,
      nunca durante edição de célula, nunca enquanto outro save roda; falha silenciosa com aviso discreto.
- [x] Exportar (PDF/CSV/XLSX/ODS) e `saveBytes` como jobs; remover `(*d.wb).clone()` de `export`.
      _(`file.exportCsv`/`file.exportHtml` de planilhas grandes já rodam em job; `export` não clona
      quando não troca de aba. PDF (pipeline de impressão) e `saveBytes` (devolve o conteúdo)
      seguem inline.)_
- [x] Unificar **todos** os caminhos de abertura (senha, drag&drop, control, `file.open`) em `open_path_async`.
      _(Feito na Rodada 5: `open_document_async` é o loader único; `open_path_async` e
      `open_bytes_async` são wrappers. Diálogo de senha e link de recentes abrem em thread
      (`retry` reabre o diálogo com senha errada). `control`/MCP fica síncrono de propósito —
      contrato de requisição/resposta dos scripts.)_
- [x] Compressão: avaliar `compression_level` menor no autosave (ex.: 1) e 6 no salvar manual;
      escrita das partes (abas) em paralelo.
      _(Nível feito: `SavePlan.compression` → `io::save_bytes_as_level` → `ZipOut::with_level`;
      autosave grava com 1, manual com 6. Escrita paralela das partes: concluído via `ZipOut::add_parts`
      com thread::scope para partes >= 1 MiB e merge_archive)._
- [x] Salvar e fechar: fechar só após o job concluir.
      _(O `file.close` que chega com um save em voo enfileira (`SheetApp::close_job_deferral`) e
      conclui em `finish_pending_close`: só fecha se o documento ainda é o ativo e o save o deixou
      limpo. Testes em `tests/background_save.rs`.)_
- **Aceite:** com o arquivo real, salvar e autosave mantêm a UI em ≥ 30 fps; salvar durante
  edição não perde a edição; matar o processo no meio do save não corrompe o arquivo original.

### Fase 3 — Cálculo em segundo plano (5–8 dias) · **resolve F9 e edições pesadas**

- [x] `Calc` rodando sobre **cópia** do `Workbook` em thread (`calc_now` hoje já clona o `wb`).
- [x] Resultado aplicado em lote ao concluir (somente valores; fórmulas não mudam no meio).
      _(Guarda por `Arc::ptr_eq` do snapshot: se a planilha mudou durante o cálculo, o resultado é
      descartado.)_
- [x] `cells_changed`: se a cascata estimada (tamanho do `dirty_closure`) > limiar (ex.: 5 k), vira job;
      abaixo disso continua inline (mantém digitação instantânea).
      _(`Calc::cascade_exceeds` conta com saída antecipada; limite `CASCADE_JOB_CELLS` = 5 000.
      Colar > 5 000 células também difere — fórmulas novas não estão no grafo, o sinal é o
      tamanho do próprio `changed`.)_
- [x] Recálculo cooperativo: avaliar em lotes com orçamento de tempo e `request_repaint`,
      atualizando as células **visíveis primeiro** (prioridade por viewport).
      _(Progresso e cancelamento já existiam; ordenação em `run_scoped` prioriza células da aba ativa/scoped
      em cada rodada de avaliação)._
- [x] Edição durante cálculo: cancelar o job e reenfileirar com o novo `revision`.
      _(`SheetApp::after_engine` cancela o job em voo e reenfileira sobre o snapshot mais novo;
      a guarda `Arc::ptr_eq` descarta resultados obsoletos.)_
- [x] Operações estruturais (`structural`) usam o mesmo caminho em vez de `recalc_all` síncrono.
      _(`DocState.defer_structural` + `recalc_pending`; só a interface liga, então CLI, testes e
      o canal de controle mantêm o contrato síncrono. `formulas.calculateNow` e o job quitam a
      dívida.)_
- Otimizações do próprio motor (independentes de thread):
  - [x] `cache de range_values` por passada.
        _(Rodada 8: 500 `SUMIF` sobre 20 mil linhas — **3 175 → 188 ms (16,9×)**.)_
  - [x] índice por intervalo para `MATCH`/`VLOOKUP` exato.
        _(Já estava ligado no despachante do avaliador; a Rodada 10 corrigiu a **chave
        numérica** do índice para a igualdade de 15 dígitos do Excel, com teste de regressão.)_
  - [x] índice para `COUNTIFS/SUMIFS` com critérios de igualdade.
        _(Rodada 11 + medição 2026-10-23: cascata 10 mil `SUMIFS` / 50 mil linhas —
        **7 083 → 94,4 ms (75×)**; baseline `=B1` × 10 mil = 10,4 ms.)_
  - [x] `rebuild` incremental do grafo no F9.
        _(Rodada 12: `ensure_graph` no-op se `graph_valid`; job clona o `Calc`; estrutural
        invalida; cascata diferida faz `update_nodes`. Medido 50 k fórmulas: rebuild 35 ms →
        ensure 4,6 µs; F9 113 → 77 ms.)_
- [x] Solver / Atingir meta / Cenários: mesmo job, com cancelar.
      _(Solver e Goal Seek rodam em `run_analysis` sobre uma Session privada, com progresso e
      cancelamento reais; o resultado entra com um único undo. `scenarioShow` é commit barato
      (coberto pelo deferimento de cascata); `scenarioSummary` é só catálogo, fica síncrono.)_
- **Aceite:** F9 no arquivo real mantém a UI responsiva e mostra "Calculando… n %";
  editar `COTACAO!B1` não congela; resultado final idêntico ao cálculo síncrono (teste de comparação).
  _(Equivalência coberta em `crates/calc/src/tests.rs`: recálculo com `hooks` × sem. O mecanismo
  do aceite está no código: F9 e cascata > 5 000 fórmulas viram job com "Calculando…" — falta
  validar no arquivo real do dono.)_

### Fase 4 — Orçamento por frame na UI (2–3 dias)

- [x] Medir `grid.rs`: cache de strings formatadas por célula visível; invalidar só ao mudar `revision`.
      _(Implementado via `GridState.text_cache` por `(uid, revision, sheet)`)._
- [x] Impressão/pré-visualização e cópia grande: gerar em job com cache.
      _(Caches de paginação `pages`/`paper` por `(uid, revision, sheet)` no grid e limite de orçamento de bytes no clipboard)._
- [x] `auto_row_height`: só linhas visíveis/alteradas, com teto.
      _(Teto de `MAX_FIT_ROWS = 1_000` em `cmd/mod.rs` no commit)._
- **Aceite:** rolagem fluida (≥ 60 fps) nas abas grandes; frame watchdog sem avisos no uso normal.

### Fase 5 — Estrutura de dados (opcional, maior, 1–2 semanas)

- [x] Armazenamento de células em blocos/linhas com `Arc` (copy-on-write fino) para que editar
      1 célula não copie 350 mil. Remove o custo do item #5 e acelera desfazer/refazer.
      _(`CellStore`: Arc interno + bandas de 64 — Rodada 14; documentado em architecture.md.)_
- [x] Desfazer baseado em _diff_ em vez de snapshot completo (menos memória).
      _(Rodada 13–14: `HistoryState::Diff` com `WorkbookDiff` — células e meta de aba; snapshot
      COW só para topologia/workbook-level / patches enormes. Rodada 15: undo Diff reusa
      `update_nodes`/`ensure_graph` — proxy 50k undo **~44 ms → ~55 µs**.)_
- **Aceite:** editar uma célula em aba de 350 mil fórmulas < 20 ms (sem contar cascata).
  _(Proxy 50k: ~1 ms / ~149 µs meta. Real `LISTA TESTE V3.xlsx`: **6,1 ms** edit / **2,3 ms** meta.)_

### Fase 6 — Guarda-corpos permanentes

- [x] Regra no `cargo xtask` (`xtask perf`): roda o corpus pesado e falha se estourar orçamento.
      _(Corpus escalável via `--rows N` / `GRIDCRAFT_BUDGET_ROWS`; o teste lista os piores e falha
      pelo `GRIDCRAFT_BUDGET_MS`.)_
- [x] Lint de revisão: proibir `std::fs`, `save_bytes*`, `recalc_all` fora de `jobs`/`engine`
      (checagem por grep no `xtask layers`).
      _(Freeze guard: proíbe `save_bytes`, `recalc_all`, `recalc_sheet`, `cells_changed`,
      `write_file`, `dbfile::save` em `crates/ui-egui/src` **e no shell desktop**
      (`apps/gridcraft/src`), e `load_document` fora de `lib.rs`.
      `std::fs::` de metadados baratos (recentes/fontes/tema/screenshots) fica permitido de
      propósito — já é threaded ou por-frame barato; documentado no próprio guard.)_
- [x] Documentar o padrão em `docs/architecture.md` e contribuir o aprendizado ao `craftrules`
      (standard "never-freeze"). _(Feito em `docs/architecture.md`, seção "Never freeze the UI".
      Pendente só a contribuição ao `craftrules`, fora deste repositório.)_

---

## 5. Critérios de aceite globais

grid-excel\LISTA TESTE V3.xlsx

| Cenário (arquivo real de 350 mil fórmulas) | Hoje                                | Meta                                            |
| ------------------------------------------ | ----------------------------------- | ----------------------------------------------- |
| Salvar (Ctrl+S)                            | UI congelada por todo o save        | UI fluida; barra de progresso                   |
| AutoSave                                   | congela a cada 2 s de edição        | invisível (job com debounce)                    |
| F9                                         | 12 s+ congelado (antes: indefinido) | UI fluida + "Calculando…"                       |
| Editar célula de entrada (`COTACAO!B1`)    | congela durante a cascata           | UI fluida; valores atualizam por prioridade     |
| Frame da UI (p95)                          | variável                            | ≤ 16 ms; nenhum frame > 100 ms no uso normal    |
| Fechar app com job rodando                 | n/a                                 | aguarda/cancela com confirmação                 |
| Falha no job                               | n/a                                 | mensagem de erro, documento intacto, sem pânico |

## 6. Riscos e decisões

| Risco                                           | Mitigação                                                                                             |
| ----------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Corrida entre edição e job (resultado obsoleto) | `revision` + snapshot imutável (`Arc`); resultado só aplica se válido                                 |
| Wasm sem threads                                | executor cooperativo por fatias; mesma API de `JobManager`                                            |
| Salvar parcial corrompe arquivo                 | gravação em `.tmp` + rename atômico                                                                   |
| Cálculo parcial mostrando valores velhos        | indicador "Calculando…"; ordem por viewport; opção de modo manual (já existe `CalcMode`)              |
| Memória (snapshots)                             | `Arc` compartilha; descartar snapshot ao fim do job                                                   |
| Panics em thread                                | `catch_unwind` no corpo do job (padrão do `Session::execute`); erro vira mensagem                     |
| Regressão de comportamento                      | teste de equivalência síncrono × assíncrono; fuzz existente (`command_sequence_fuzz`) cobre undo/redo |

## 7. Ordem recomendada e esforço

1. **Fase 0 + 1** (infra e medição) — base de tudo.
2. **Fase 2** — maior ganho imediato para o usuário (**Salvar + AutoSave**).
3. **Fase 3** — F9 e edição com cascata.
4. Fases 4–6 conforme os números da Fase 0.

Esforço total estimado: ~3–4 semanas para fases 0–4; fase 5 é opcional e independente.

## 8. Decisões que preciso do dono do projeto

1. **Edição durante o salvamento**: permitir (snapshot) ou bloquear com modal? _(Recomendado: permitir.)_
2. **AutoSave**: manter ligado por padrão? Intervalo/debounce desejado? _(Sugestão: 5 s após a última edição.)_
3. **Prioridade**: começar por Salvar/AutoSave (Fase 2) ou por F9/cálculo (Fase 3)?
4. **Wasm**: o app web precisa ter o mesmo comportamento já na Fase 2, ou pode vir depois?
