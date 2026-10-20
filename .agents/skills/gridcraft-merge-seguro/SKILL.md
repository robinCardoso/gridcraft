---
name: gridcraft-merge-seguro
description: >-
  Guia especialista e protocolo seguro para sincronizar e mesclar atualizações da ramificação oficial (origin/main)
  na ramificação personalizada (minhas-personalizacoes) do GridCraft, resolvendo conflitos sem perda de código e sem regressões.
---

# Protocolo Especialista de Merge Seguro no GridCraft

Este guia define o procedimento operacional padrão para incorporar novidades e correções do repositório oficial (`origin/main`) na ramificação local (`minhas-personalizacoes`), garantindo que nenhuma funcionalidade personalizada seja perdida ou sofra regressão.

---

## 1. Regra Fundamental
> [!IMPORTANT]
> A ramificação `minhas-personalizacoes` contém recursos exclusivos vitais:
> 1. Suporte nativo a banco de dados SQLite (`.db`).
> 2. Fórmulas completas em Português (`SOMA`, `PROCV`, `SE`, separador `;`).
> 3. Otimizações de dependência $O(N+M)$ e busca $O(1)$ para `PROCV`.
> 4. "Colar como Texto" (preservação de zeros à esquerda com formato `@`).
> 5. Cancelamento de cópia e desativação do botão "Colar" com <kbd>Esc</kbd>.
> 
> **Nenhum merge com o código oficial pode sobrescrever ou descartar essas funcionalidades.**

---

## 2. Fase 1: Ponto de Restauração e Fetch (Segurança Inicial)

Antes de qualquer ação de mesclagem:

1. **Garantir diretório de trabalho limpo:**
   ```bash
   git status
   ```
2. **Criar branch de backup com identificador de versão/data:**
   ```bash
   git branch backup-antes-do-merge
   ```
   *Se qualquer conflito inesperado ocorrer, é possível voltar ao estado anterior com `git reset --hard backup-antes-do-merge`.*

3. **Buscar atualizações remotas (requer acesso de rede):**
   ```bash
   git fetch origin
   ```

---

## 3. Fase 2: Análise Prévia (Dry-Run)

1. **Verificar os commits novos da `origin/main`:**
   ```bash
   git log minhas-personalizacoes..origin/main --oneline
   ```
2. **Verificar os arquivos modificados por ambas as partes:**
   Identifique se arquivos críticos foram tocados simultaneamente por ambos os lados.

---

## 4. Fase 3: Playbook de Resolução de Conflitos Conhecidos

Ao executar:
```bash
git merge origin/main
```

Se houver conflitos, siga este manual para os arquivos recorrentes:

### A. Conflito em `crates/engine/src/cmd/file.rs`:
- **Nosso código:** Bloco `is_db_path(&name)` para carregar bancos SQLite `.db`, comando `file.importXlsxToDb`.
- **Código oficial:** Novos parâmetros como `csvAsText` em `open_bytes_with`.
- **Como resolver:** Mantenha a verificação `if crate::dbfile::is_db_path(&name)` e, no ramo `else`, utilize a versão atualizada da função oficial (`open_bytes_with(&name, &bytes, password, csvAsText)`).

### B. Conflito em `crates/ui-egui/src/grid.rs`:
- **Nosso código:** Caches de performance em `GridState` (`line_index_cache`, `used_range_cache`, `cf_cache`), cancelamento `app.cancel_copy()` nos eventos de `Key::Escape` e `Key::Enter`, e desativação de itens nos menus de contexto (`add_enabled(!is_paste || can_paste, ...)`).
- **Código oficial:** Recursos de touch pan (`touch_pan: bool`) ou novos atalhos de linha inteira.
- **Como resolver:** Conserve todos os campos de cache e adicione o campo oficial (`touch_pan: bool`) na struct. Mantenha os métodos de cancelamento com <kbd>Esc</kbd>.

### C. Conflito em `crates/ui-egui/src/ribbon.rs` e `widgets.rs`:
- **Nosso código:** Botão "Colar" com estado desabilitado (`big_button_enabled`, `can_paste`) e opção "Colar como Texto" (`edit.pasteAsText`).
- **Como resolver:** Mantenha a chamada a `big_button_enabled` e preserve os itens de menu adicionais.

### D. Conflito em `crates/ui-egui/src/i18n.rs` e `tests/formula_locale.rs`:
- **Nosso código:** `FormulaLocale::Pt` associado a `Language::PtBr` e suporte a fórmulas como `SOMA(1,5;2,25)`.
- **Como resolver:** Mantenha `Self::PtBr => FormulaLocale::Pt` e nunca reverta para o fallback inglês.

---

## 5. Fase 4: Bateria Obrigatória de Testes de Regressão

Após resolver os conflitos e adicionar com `git add`, execute obrigatoriamente:

1. **Testes de Clipboard, Fórmulas e Widgets:**
   ```powershell
   & "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo test -p gridcraft-ui-egui --test clipboard --test formula_locale --test widgets
   ```
2. **Testes do Banco de Dados SQLite:**
   ```powershell
   & "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo test -p gridcraft-db
   ```
3. **Testes do Motor Principal:**
   ```powershell
   & "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo test -p gridcraft-engine
   ```
4. **Build do Executável Release:**
   ```powershell
   & "C:\Users\Robson-PC\.gemini\antigravity-ide\brain\255379b2-b0af-4555-a53a-5d7b9b2f0656\scratch\run_cargo.bat" cargo build --release --offline -p gridcraft
   ```

---

## 6. Fase 5: Finalização ou Cancelamento

- **Se todos os testes passarem com 100% de sucesso:**
  ```bash
  git commit -m "Merge origin/main into minhas-personalizacoes (descrição das melhorias integradas)"
  ```
- **Se algo quebrar e não puder ser resolvido de imediato:**
  ```bash
  git merge --abort
  ```
  *O repositório volta instantaneamente ao estado seguro pré-merge.*
