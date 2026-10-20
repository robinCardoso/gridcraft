---
name: gridcraft-clipboard
description: >-
  Especificação técnica, regras de negócio e prevenção de regressões para o sistema de
  Área de Transferência (Clipboard), modos de cópia/corte, cancelamento com Esc e "Colar como Texto" no GridCraft.
---

# Sistema de Clipboard e Colagem no GridCraft

Este documento detalha o funcionamento de cópia, recorte e colagem no GridCraft, o comportamento do botão "Colar" na Ribbon, menus de contexto e como prevenir regressões em alterações futuras.

---

## 1. Arquitetura de Estado (`crates/ui-egui/src/lib.rs`)

O estado do clipboard é gerenciado na estrutura [`SheetApp`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/lib.rs):

```rust
pub struct SheetApp {
    /// Texto do clipboard do sistema no momento em que a cópia foi feita.
    clipboard_text_at_copy: Option<String>,

    /// Texto em memória de um recorte (Cut). Quando colado uma vez, é consumido.
    cut_text: Option<String>,

    /// Texto do clipboard registrado quando o usuário cancela a cópia (via Esc ou Enter).
    /// Enquanto o clipboard do sistema mantiver esse mesmo texto, "Colar" fica desativado.
    cancelled_text: Option<String>,

    // ...
}
```

E no motor (`session`):
- `session.clipboard`: armazena as células copiadas (valores, fórmulas, formatações e referências) para renderizar as bordas tracejadas ("marching ants").
- `session.format_painter`: armazena o pincel de formatação.

---

## 2. Fluxo de Cancelamento com <kbd>Esc</kbd> e <kbd>Enter</kbd>

### Como funciona no Excel e no GridCraft:
1. Ao copiar (`Ctrl+C`), `session.clipboard` é preenchido e as "marching ants" aparecem.
2. Ao pressionar <kbd>Esc</kbd> (ou <kbd>Enter</kbd> para colar e encerrar):
   - [`cancel_copy(&mut self)`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/lib.rs#L432) é chamado.
   - O texto atual é salvo em `cancelled_text`.
   - `session.clipboard` é limpo (`None`), fazendo sumir a borda tracejada.
3. Consulta de disponibilidade para colagem:
   ```rust
   pub fn can_paste(&self) -> bool {
       self.session.clipboard.is_some() || (self.cancelled_text.is_none() && self.cut_text.is_none())
   }
   ```
4. Se o usuário tentar colar via <kbd>Ctrl</kbd>+<kbd>V</kbd> ou clicar no botão "Colar":
   - O método `paste_external` detecta que o texto confere com `cancelled_text` e emite a notificação `"Nothing to paste."` sem alterar a grade.

---

## 3. Renderização Visual do Botão "Colar" (`crates/ui-egui/src/widgets.rs`)

### Regra de Ouro da UI:
O botão "Colar" é renderizado via [`big_button_enabled`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/widgets.rs#L50):
- **Quando habilitado (`enabled == true`)**: cores normais (`t.text`), hover e clique destacados.
- **Quando desabilitado (`enabled == false`)**:
  - Ícone e texto desenhados em tom opaco/desabilitado (`t.text_disabled`).
  - Chevron do menu suspenso em tom desabilitado.
  - Hover e pressed são suprimidos.
  - **Atenção:** Mantém `Sense::click()` ativo para que cliques no botão possam acionar a ação de aviso `"Nothing to paste."` sem bloquear o harness de testes do egui.

---

## 4. "Colar como Texto" (Preservação de Zeros à Esquerda)

### Problema resolvido:
Valores como `0151510` ou códigos de barras colados em células no formato `Geral` perdem os zeros à esquerda virando números (`151510`).

### Implementação:
- Função [`paste_as_text(&mut self)`](file:///c:/Users/Robson-PC/.antigravity/projetos/grif-excel/gridcraft/crates/ui-egui/src/lib.rs#L490).
- Aplica o formato textual `@` (`home.numberFormat` com `"@"`) antes ou durante a colagem via `edit.paste` com parâmetro `asText: true`.
- Disponível no menu suspenso do botão "Colar" na Ribbon e no menu de contexto do botão direito da célula.

---

## 5. ARMADILHAS E REGRAS PARA EVITAR REGRESSÕES (GOTCHAS)

> [!CAUTION]
> **NUNCA leia o clipboard do sistema dentro de `can_paste()` ou no loop de renderização (`home()`)!**
> A suite de testes em `crates/ui-egui/tests/clipboard.rs` valida explicitamente:
> `assert_eq!(clipboard.borrow().1, 1, "one clipboard read per click");`
> Chamar `services.paste_text` a cada frame faz milhares de chamadas por segundo, degrada a CPU e quebra os testes imediatamente.

> [!IMPORTANT]
> **Sempre valide com a suite de testes:**
> ```powershell
> & "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo test -p gridcraft-ui-egui --test clipboard --test widgets
> ```
