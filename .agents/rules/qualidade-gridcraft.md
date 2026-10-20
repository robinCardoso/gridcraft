---
trigger: model_decision
description: Regras de qualidade de código, prevenção de regressões e testes no GridCraft
---

# Regras de Qualidade e Prevenção de Regressões no GridCraft

Sempre que estiver modificando código no projeto GridCraft, siga rigorosamente estas regras:

## 1. Regra de Ouro da Interface Gráfica (Performance e Testes)
- **NUNCA** consulte o clipboard do sistema (`services.paste_text` ou `system_clipboard_text()`) dentro de métodos de desenho a cada frame (como `home()`, `ui()`, `widgets.rs` ou `can_paste()`).
- O estado de botões e menus deve ser baseado em campos cacheados da aplicação (`self.session.clipboard`, `self.cancelled_text`, `self.cut_text`).
- O desrespeito a isso causa consumo excessivo de CPU e quebra os testes unitários (`assert_eq!(reads, 1)`).

## 2. Fórmulas e Localização
- Em português (`PtBr`), os nomes de função usam aliases em português (`SOMA`, `PROCV`, `SE`, `MÉDIA`) e o separador de lista é `;` (ponto-e-vírgula). O caractere `,` é exclusivamente separador decimal.
- Arquivos salvos e a API canônica do motor sempre preservam os nomes em inglês (`SUM`, `VLOOKUP`).

## 3. Validação com Testes
- Sempre execute os testes do crate modificado antes de finalizar tarefas:
  - Clipboard: `cargo test -p gridcraft-ui-egui --test clipboard`
  - Widgets: `cargo test -p gridcraft-ui-egui --test widgets`
  - Fórmulas: `cargo test -p gridcraft-ui-egui --test formula_locale`
- No Windows, utilize o script `run_cargo.bat` configurado com o ambiente MSVC.

## 4. Política de Commits
- Todo commit deve ser direcionado para a ramificação `minhas-personalizacoes`.
