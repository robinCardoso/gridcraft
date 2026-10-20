---
name: gridcraft-arquitetura
description: >-
  Guia de arquitetura, estrutura de crates, ciclo de vida da interface egui e procedimentos
  de build/testes do GridCraft para evitar regressões e guiar novos desenvolvimentos.
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
