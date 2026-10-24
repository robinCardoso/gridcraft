---
name: gridcraft-calc-engine
description: >-
  Guia do motor de cálculo, recalc_on_load e recalc_pending ao abrir XLSX, localização PT
  (SOMA, PROCV, SE), otimizações O(N+M), PROCV hash e barra de progresso do GridCraft.
---

# Motor de Cálculo e Localização de Fórmulas no GridCraft

Este documento detalha o sistema de cálculo, as otimizações de performance aplicadas e como funciona o suporte a fórmulas em Português no GridCraft.

---

## 1. Localização de Fórmulas em Português (`FormulaLocale::Pt`)

O GridCraft possui suporte a fórmulas em português (`crates/formula/src/input.rs`, `FormulaLocale::Pt`):

- **Separador de argumentos:** `;` (ponto-e-vírgula). Ex: `=SOMA(A1; B1)` ou `=SE(A1>0; "Sim"; "Não")`.
- **Separador decimal:** `,` (vírgula). Ex: `1,5` ou `2,25`.
- **Separador de colunas de matriz:** `\` (barra invertida). Ex: `{1\2;3\4}`.
- **Aliases traduzidos:**
  - `SUM` -> `SOMA`
  - `VLOOKUP` -> `PROCV`
  - `HLOOKUP` -> `PROCH`
  - `XLOOKUP` -> `PROCX`
  - `MATCH` -> `CORRESP`
  - `INDEX` -> `ÍNDICE` / `INDICE`
  - `AVERAGE` -> `MÉDIA` / `MEDIA`
  - `IF` -> `SE`
  - `IFS` -> `SES`
  - `IFERROR` -> `SEERRO`
  - `COUNT` -> `CONT.NÚM` / `CONT.NUM`
  - `COUNTA` -> `CONT.VALORES`
  - `COUNTIF` -> `CONT.SE`
  - `COUNTIFS` -> `CONT.SES`
  - `SUMIF` -> `SOMASE`
  - `SUMIFS` -> `SOMASES`
  - `TRUE` -> `VERDADEIRO`
  - `FALSE` -> `FALSO`

### Armadilha / Como evitar regressão:
- Ao escrever testes ou interagir com fórmulas em `Language::PtBr`, **nunca** use vírgula como separador de argumentos (ex: `=SUM(1,2)`). O parser de português interpreta `1,2` como o número float `1.2`. Sempre utilize `;` como separador de listas.
- Arquivos salvos em disco (XLSX) sempre persistem os nomes canônicos em inglês (`SUM`, `VLOOKUP`). A tradução ocorre apenas na camada de entrada e visualização do usuário (`input.rs` / `formula_locale.rs`).

---

## 2. Recálculo ao abrir (`recalc_on_load`) — contrato anti-regressão

**Código:** `crates/calc/src/recalc.rs` (`recalc_on_load`, `formulas_pending_eval`), `crates/engine/src/lib.rs` (`DocState::new`).

1. **`needs_formula_eval`:** fórmula ainda sem resultado confiável = `Value::Empty` ou `Text("")` (cache vazio de PROCV/SEERRO no XLSX). Valores numéricos/texto salvos pelo Excel **não** são recalculados na abertura (rápido).
2. **`recalc_on_load`:** reconstrói o grafo; avalia só o que `needs_formula_eval`. Se **> 1.000** células, roda no máximo **1.000** na **aba ativa** (+ fórmulas de matriz dinâmica noutras abas).
3. **Após o load:** se `Calc::formulas_pending_eval(wb) > 0` → **`DocState.recalc_pending = true`**. A UI deve disparar job F9 em background (`ui-egui`: `after_engine` + `calc_job_attempt(..., force: true)`). **Regressão clássica:** remover esse flag e deixar ~640k fórmulas sem valor visível.
4. **CLI/testes sem UI:** `recalc_pending` fica true; usar `formulas.calculateNow` ou `recalc_document` explicitamente se o teste precisar de valores completos.

**Testes:**

- `cargo test -p gridcraft-calc recalc_on_load_leaves_pending`
- `cargo test -p gridcraft-engine lista_v3_open_formula_coverage` (com `LISTA TESTE V3.xlsx` no repo)

Detalhes de fechar janela / salvar: skill **`gridcraft-arquitetura`** §5.2.

---

## 3. Otimizações de Recálculo

### A. Otimização de Dependências Dirty Closure:
- Localização: `crates/calc/src/recalc.rs` (grafo e dirty closure).
- A verificação de dependências entre colunas inteiras foi otimizada de $O(N \times M)$ para $O(N + M)$ com conjuntos de colunas inteiras (`whole_col_deps`) e eliminação de duplicatas de colunas.

### B. Índice Hash $O(1)$ para PROCV / VLOOKUP Exato:
- Em buscas exatas (`range_lookup = FALSE` ou `0`), a primeira coluna da matriz de busca é indexada em um mapa hash na primeira consulta, permitindo buscas instantâneas $O(1)$ em listas com dezenas de milhares de linhas.

---

## 4. Indicador e Barra de Progresso na Barra de Status

- Localização: `crates/ui-egui/src/status_bar.rs` e `SheetApp::calc_status` em `crates/ui-egui/src/lib.rs`.
- Exibe o progresso em porcentagem e quantidade de fórmulas quando há recálculos pesados em andamento.
- **Auto-ocultação:** Quando o cálculo atinge 100%, o indicador permanece visível por 2.5 segundos e depois some automaticamente para não poluir a interface.
