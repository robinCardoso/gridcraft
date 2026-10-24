---
name: gridcraft-arquitetura
description: >-
  Guia de arquitetura, estrutura de crates, ciclo de vida da interface egui, contratos
  anti-regressão (abrir XLSX, recálculo em background, fechar/salvar) e build/testes do GridCraft.
---

# Arquitetura e Guia Geral do GridCraft

Este documento serve como mapa de referência para entender como o GridCraft é estruturado, como compilar e testar sem quebrar o ambiente e quais são as responsabilidades de cada crate.

---

## 1. Estrutura de Crates

O GridCraft é um workspace Rust modularizado:

| Crate / Caminho | Responsabilidade |
| :--- | :--- |
| **`apps/gridcraft`** | Binário executável desktop (inicialização da janela e loop principal do eframe). |
| **`crates/engine`** | Núcleo do motor de planilhas: modelo de dados (`Workbook`, `Worksheet`), grafo de dependências e recálculo, journaling de comandos (Undo/Redo), importação/exportação XLSX. |
| **`crates/formula`** | Parser Léxico e Sintático (AST) de fórmulas, avaliação de expressões, regras de formatação e localização (`FormulaLocale::En`, `FormulaLocale::Es`, `FormulaLocale::Pt`). |
| **`crates/functions`** | Implementação das fórmulas individuais do Excel (ex: `SOMA`, `PROCV`, `SE`, `CONT.SE`, `CRITERIA`, matemática, datas, texto). |
| **`crates/ui-egui`** | Interface gráfica construída em `egui`: Ribbon (abas e botões), Grade de células (`grid.rs`), cabeçalhos, barra de fórmulas, menus de contexto, diálogos e widgets customizados. |
| **`crates/core`** | Tipos primitivos compartilhados (`CellRef`, `Value`, `CellError`, formatação, coordenadas). |

---

## 2. Ciclo de Vida da Interface (`crates/ui-egui`)

A interface utiliza o framework imediato `egui`:
1. **`SheetApp::logic(&mut self, ctx: &egui::Context)`**:
   - Processa mensagens assíncronas do backend (ex: cálculo concluído, abertura de arquivos).
   - Atualiza animações e estados de transição.
2. **`SheetApp::ui(&mut self, ui: &mut egui::Ui)`**:
   - Renderiza a barra de título, Ribbon (`src/ribbon.rs`), barra de fórmulas (`src/formula_bar.rs`), grade de células (`src/grid.rs`) e barra de status (`src/status_bar.rs`).
   - **REGRA DE OURO DE PERFORMANCE:** **Nunca** faça I/O síncrono, leitura de clipboard do sistema operacional ou operações pesadas diretamente dentro da renderização da UI (isso causa travamentos a 60 FPS e quebra asserções nos testes automatizados).

---

## 3. Ambiente de Compilação e Testes (Windows)

No Windows, o compilador MSVC precisa do ambiente de ferramentas configurado para carregar o linker correto (`vcvars64.bat`).

### Executando comandos Cargo com segurança:
Utilize o wrapper batch localizado nos artefatos da sessão:
```powershell
& "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo test -p gridcraft-ui-egui
```
Ou ative o ambiente do Visual Studio antes de rodar o `cargo`:
```cmd
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cargo test -p gridcraft-ui-egui
```

### Comandos Comuns:
- **Testes de Clipboard:** `cargo test -p gridcraft-ui-egui --test clipboard`
- **Testes de Widgets da UI:** `cargo test -p gridcraft-ui-egui --test widgets`
- **Testes de Fórmulas e Localização:** `cargo test -p gridcraft-ui-egui --test formula_locale`
- **Compilar Release do Executável:** `cargo build --release -p gridcraft`

---

## 4. Política de Ramificações e Commits

- Todas as customizações e melhorias locais devem permanecer na branch **`minhas-personalizacoes`**.
- Antes de commitar:
  1. Execute os testes do crate alterado.
  2. Garanta que o `cargo check` ou `cargo test` passe com 100% de sucesso.
  3. Crie mensagens de commit descritivas seguindo o padrão convencional (ex: `fix(clipboard): ...`, `feat(engine): ...`).

---

## 5. Contratos anti-regressão (obrigatório ao mexer em abrir/fechar/calc)

Use esta seção quando alterar **`file.open`**, **`file.close`**, **`DocState::new`**, **`recalc_on_load`**, **`SheetApp::after_engine`**, **`dialogs::saveChanges`** ou o fechamento da janela em **`apps/gridcraft/src/main.rs`**.

### 5.1 Abrir workbook (XLSX grande)

**Comportamento esperado (estilo Excel):**

1. `load_document` / `DocState::new` chama `calc.recalc_on_load`.
2. `recalc_on_load` só calcula **sincronamente** fórmulas sem cache confiável; se forem **> 1.000**, limita à **aba ativa** (mais fórmulas de matriz dinâmica).
3. Se ainda restarem fórmulas com `Cell::needs_formula_eval()` (`Value::Empty` ou `Text("")`), **`DocState.recalc_pending` deve ser `true`** (`Calc::formulas_pending_eval`).
4. Na UI nativa, `SheetApp::after_engine` enfileira job de cálculo (`calc_job_attempt(true, true)`) enquanto `recalc_pending` — o usuário vê progresso na barra de status; **não** deixar a grade “vazia” sem job em livros com dezenas/centenas de mil fórmulas.
5. `file.open` pode expor `"recalcPending": true` no JSON de `install_document`.

**Sintoma de regressão:** abrir `LISTA TESTE V3.xlsx` e ver colunas calculadas (LISTA_PRECO, ORCAMENTO) todas em branco, sem “Calculando…”.

**Testes de referência:**

```powershell
cd gridcraft
cargo test -p gridcraft-calc recalc_on_load_leaves_pending -- --nocapture
cargo test -p gridcraft-engine lista_v3_open_formula_coverage -- --nocapture
```

(O segundo teste exige `LISTA TESTE V3.xlsx` na raiz do repo ou em `grid-excel\`.)

**Não fazer:**

- Remover `recalc_pending` em `DocState::new` “para simplificar”.
- Assumir que `recalc_on_load` recalcula o livro inteiro (não recalcula em arquivos enormes).
- Trocar aba (`wb_switch_sheet`) esperando recálculo automático da aba — troca de aba **não** chama `recalc_sheet` (performance); o job global ou F9 pagam a dívida.

### 5.2 Fechar livro vs fechar janela

**Dois fluxos distintos:**

| Ação | Comportamento |
| :--- | :--- |
| **Arquivo → Fechar**, **Ctrl+F4** | Comando `file.close` no documento **ativo**. |
| **X da janela**, **Alt+F4**, **Sair**, **`app.quit`**, Quit macOS | `SheetApp::handle_viewport_close_request` → percorre livros sujos com o **mesmo** `file.close` → só então `ViewportCommand::Close`. |

**Comportamento de `file.close` (engine):**

- Se `d.is_dirty() && !force` → enfileira diálogo **`saveChanges`** (`UiRequest::Dialog`) e retorna `{"closed": false, "dirty": true}`.
- **`force: true`** só após o usuário escolher **Não salvar** ou após save bem-sucedido.
- Antes de `file.close`, a UI deve **`commit_edit`** (célula em edição conta como alteração).

**Comportamento de `saveChanges` (UI):**

- **Salvar** → `file.save` (+ save em background se grande) → `file.close` force quando limpo ou save enfileirado (`pending_close_uid`).
- **Não salvar** → `file.close` com `force: true`.
- **Cancelar** → fecha o diálogo; se era quit da janela, **`pending_viewport_close = false`** (não fechar o app).

**Sintoma de regressão:** alterar A1, clicar no **X** e o app encerrar sem pergunta; ou **Arquivo → Fechar** sem diálogo com livro sujo.

**Teste de referência:**

```powershell
cargo test -p gridcraft-ui-egui --test file_menu save_actions_include_the_cell_draft_and_close_keeps_the_unsaved_prompt -- --nocapture
```

(Fluxo Backstage **Fechar**; cobre o mesmo diálogo `saveChanges` que o engine usa no quit.)

**Pontos de integração (não quebrar o encadeamento):**

- `apps/gridcraft/src/main.rs`: `close_requested` → `handle_viewport_close_request` + `CancelClose`, **exceto** quando `viewport_quit_ready_to_close()` (quit já confirmado; senão o `Close` programático é cancelado de novo).
- `SheetApp::continue_viewport_quit` no fim de `ui()` e quando `session.active()` fica vazio com `pending_viewport_close` (não criar `Book1` ao sair). **Não** zerar `pending_viewport_close` ao enviar `ViewportCommand::Close` — só **Cancelar** no diálogo; senão o `logic()` do frame seguinte abre Book1 vazio em vez de encerrar.
- `apps/gridcraft/src/apple_events.rs`: `Event::Quit` → `handle_viewport_close_request`, não `Close` direto.

### 5.3 Checklist rápido antes de PR/commit (abrir/fechar/calc)

1. Alterou engine de abertura → rodar testes `gridcraft-calc` + `lista_v3_open_formula_coverage` se existir o XLSX.
2. Alterou UI de fechar/salvar → `cargo test -p gridcraft-ui-egui --test file_menu`.
3. Alterou deferral / jobs → `cargo test -p gridcraft-ui-egui --test background_save` e testes `pedido_*` / `mb006_*` no engine se tocaram `recalc_pending`.
4. Smoke manual: editar célula → **X** (pergunta salvar); **Arquivo → Fechar** (mesma pergunta); abrir planilha grande → barra **Calculando…** até convergir.

### 5.4 Perfil LISTA TESTE V3 — E1 / COTACAO (2026-10-23)

Teste reprodutível: `crates/engine/tests/lista_v3_e1_profile.rs`

```powershell
$env:GRIDCRAFT_CALC_STATS='1'
cargo test -p gridcraft-engine lista_v3_cotacao_calculate_sheet_profile --release -- --nocapture
```

**Medições (release, arquivo na raiz do repo):**

| Métrica | Valor |
| :--- | :--- |
| Fórmulas na aba COTACAO | ~20 394 |
| Nós no grafo (livro inteiro) | ~640 561 |
| **E1** | **valor texto** (ex. referência), não fórmula |
| `calculateSheet` (preencher COTACAO) | **~17 486 fórmulas / ~24 s** (release; Q4 + ISNA/XMATCH batch + cache LET `cadOk`/`naoServem`) |
| Editar **D16** (uma Referencia) | **7 fórmulas / ~87 ms** (LET ~84 ms em E16+L16) |
| **`equality hit`** nos stats | **>0** (`build≈1`) — índice coluna=código + `ReferenceKey` (ORCAMENTO `achou`) |
| Funções dominantes (amostra) | **IF**, **MIN**, **N**; **FILTER ~1×/pass** (bloco fornecedor cacheado); **LET** quase só no cold path |

**Hipótese confirmada:** **H1 + H4** (`plano-desempenho-calculo.md`) — custo por linha do ORCAMENTO (LET + match em LISTA_PRECO), não I/O de arquivo. Cauda: algumas linhas **>600 ms** (E656/L656).

**Regressão a evitar:** `equality hit` voltar a 0 após mudanças em lookup/`MATCH(TRUE,…)`.
