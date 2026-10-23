# Personalizações intencionais — checklist de regressão

Documento **obrigatório** no gate de overlap/vereditos da skill
[`gridcraft-merge-seguro`](../.agents/skills/gridcraft-merge-seguro/SKILL.md).

Antes de `git merge origin/main` (e ao julgar cada veredito em overlap), o agente **deve**
consultar esta lista. Se um path do overlap tocar a área de um item abaixo, o veredito
padrão é **Manter o nosso** ou **Combinar** (nunca “Pegar o do main” sem plano que preserve
o comportamento).

Detalhe técnico / histórico: [`plano-desempenho-calculo.md`](./plano-desempenho-calculo.md)
§7.3, [`plano-responsividade.md`](./plano-responsividade.md) (handoff).

---

## Recursos estáveis (Regra Fundamental da skill)

| Recurso | Onde costuma viver | Não perder |
|---------|-------------------|------------|
| SQLite `.db` | `engine` `dbfile` / `cmd/file.rs` | `is_db_path`, `file.importXlsxToDb` |
| Fórmulas PT (`SOMA`, `;`) | `i18n`, `formula_locale`, locale | `FormulaLocale::Pt` em `PtBr` |
| Colar como Texto (`@`) | `edit.pasteAsText`, ribbon/menus | zeros à esquerda |
| Esc cancela cópia / desativa Colar | `grid.rs`, clipboard UI | `cancel_copy`, `can_paste` |
| Caches de grelha | `GridState` | `line_index_cache`, `used_range_cache`, `cf_cache` |

---

## Regressões recentes (obrigatório no gate)

Cada item é uma personalização / correção **intencional**. Upstream pode “parecer certo”
e ainda assim reintroduzir o bug.

| # | Intenção | Sintoma se regredir | Âncoras (grep / paths) | Testes / commits |
|---|----------|---------------------|------------------------|------------------|
| 1 | **`needs_formula_eval` + cache `Text("")`** | Linhas do corpo da tabela não recalculam (PROCV fica em branco); só expand “funciona” | `needs_formula_eval`, `Text("")`, `touch_table_formula_rows`, `Value::is_empty` vs `Empty` | `pedido_procv_repro`; `c73117e` |
| 2 | **`apply_job` bump de `revision`** | Após job de calc, `text_cache` da UI mantém `""` stale; editar outra célula “acordava” o paint | `apply_job`, `d.revision += 1`, `text_cache` | `pedido_paste_recalc`; `5b9ff9e` |
| 3 | **`resolve_struct` / precedents H9** | Edit pequeno em structured ref → dirty enorme (~600k); progresso mentiroso | `resolve_struct`, `Expr::Struct`, `precedents`, `graph.dynamic` | calc/engine tests H9; planos § structured refs |
| 4 | **`invalidate_graph` estrutural** | Expand/tabela após edit estrutural síncrono sem rebuild → deps erradas | `invalidate_graph`, branch estrutural em `commit` | `db8b630` |
| 5 | **`recalc_cascade` seeds** | Defer estrutural sem seeds → cascata vazia / viewport sem fórmulas da linha colada | `recalc_cascade`, `defer_structural`, `CASCADE_JOB_CELLS` | `55f3426`; `pedido_procv_repro` |
| 6 | **Clipboard consume after paste** | Colar de novo cola o mesmo corte/cópia; botão Colar fica ativo sem nova cópia | `consume` clipboard, `clear_clipboard`, paste after success | `clipboard` UI tests; `5ebccdc` |
| 7 | **Yield / coop** | Barra de progresso “morta”; Cancel imprevisível em fórmulas longas | `PassHost::coop`, `set_coop_interval_ms`, yield ~75 ms | planos Q1; calc hooks |
| 8 | **Automatic sem truncate >1000** | Automatic corta dirty em 1000 e deixa o livro inconsistente | `needed.truncate(1_000)`, Automatic vs limit explícito | `recalc.rs`; planos “Sem truncate >1000” |
| 9 | **`recalc_keys` na cascata adiada** | Com `recalc_pending`, Referencia já muda mas Descricao/Linha da mesma linha ficam no lookup anterior (ex.: MB006 + Caterpillar) | `recalc_keys`, `recalc_pending`, `touch_table_formula_rows_for_input_edits` | `mb006_same_key`; `same_lookup_key_two_rows_same_match_index` |

### Como usar no gate

1. Montar a interseção de paths (ver [reference.md](../.agents/skills/gridcraft-merge-seguro/reference.md)).
2. Para cada path, marcar se intersecta alguma âncora desta tabela (grep no diff `$BASE..origin/main` e `$BASE..HEAD`).
3. Na coluna Motivo do veredito, citar o **#** da checklist quando aplicável.
4. Na bateria pós-merge, preferir os testes da coluna “Testes” dos itens tocados, além da bateria padrão da skill.

---

## Atualização deste arquivo

Atualizar quando fechar um bug de personalização que possa ser silenciado por auto-merge
do `main`. Não listar todo o histórico do repo — só intenções que o gate precisa lembrar.
