---
name: gridcraft-calc-engine
description: >-
  Guia do motor de cálculo, localização de fórmulas em português (SOMA, PROCV, SE), otimizações
  de dependências O(N+M), hash index para PROCV e barra de progresso do GridCraft.
---

# Motor de Cálculo e Localização de Fórmulas no GridCraft

Este documento detalha o sistema de cálculo, as otimizações de performance aplicadas e como funciona o suporte a fórmulas em Português no GridCraft.

---

## 1. Localização de Fórmulas em Português (`FormulaLocale::Pt`)

O GridCraft possui suporte a fórmulas em português ([`FormulaLocale::Pt`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/formula/src/input.rs)):

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

## 2. Otimizações de Recálculo

### A. Otimização de Dependências Dirty Closure:
- Localização: [`crates/engine/src/calc.rs`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/engine/src/calc.rs).
- A verificação de dependências entre colunas inteiras foi otimizada de $O(N \times M)$ para $O(N + M)$ com conjuntos de colunas inteiras (`whole_col_deps`) e eliminação de duplicatas de colunas.

### B. Índice Hash $O(1)$ para PROCV / VLOOKUP Exato:
- Em buscas exatas (`range_lookup = FALSE` ou `0`), a primeira coluna da matriz de busca é indexada em um mapa hash na primeira consulta, permitindo buscas instantâneas $O(1)$ em listas com dezenas de milhares de linhas.

---

## 3. Indicador e Barra de Progresso na Barra de Status

- Localização: [`crates/ui-egui/src/status_bar.rs`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/status_bar.rs) e [`SheetApp::calc_status`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/lib.rs).
- Exibe o progresso em porcentagem e quantidade de fórmulas quando há recálculos pesados em andamento.
- **Auto-ocultação:** Quando o cálculo atinge 100%, o indicador permanece visível por 2.5 segundos e depois some automaticamente para não poluir a interface.
