---
name: gridcraft-merge-seguro
description: >-
  Guia especialista e protocolo seguro para sincronizar e mesclar atualizações da ramificação oficial (origin/main)
  na ramificação personalizada (minhas-personalizacoes) do GridCraft, com gate de overlap/vereditos para não
  sobrescrever personalizações melhores, resolvendo conflitos sem perda de código e sem regressões.
---

# Protocolo Especialista de Merge Seguro no GridCraft

Este guia define o procedimento operacional padrão para incorporar novidades e correções do repositório oficial (`origin/main`) na ramificação local (`minhas-personalizacoes`), garantindo que nenhuma funcionalidade personalizada seja perdida, misturada sem julgamento ou sofra regressão.

Arquivos irmãos: [reference.md](reference.md) (comandos de overlap, modelo de tabela de vereditos).

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
>
> Personalizações **melhores ou intencionais** devem ser **mantidas ou aprimoradas** (trazer o ganho do `main` sobre a base nossa). Não misturar nem aceitar o lado do `main` por conveniência de auto-merge.

Documento opcional recomendado: `docs/personalizacoes.md` (ou equivalente) listando divergências intencionais e o motivo. Consulte-o ao julgar vereditos. **Não criar stub** só por política desta skill — só escrever/atualizar se o usuário pedir ou se já existir e precisar refletir um veredito novo.

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

## 3. Fase 2: Gate obrigatório de overlap e vereditos (ANTES do merge)

> [!CAUTION]
> Esta fase é **obrigatória** após o fetch e **antes** de `git merge`. Sem tabela de vereditos completa para o overlap, o merge **não** pode seguir em modo automático.

Upstream padrão: `origin/main` (ou o remoto/upstream acordado). Branch de personalizações: `minhas-personalizacoes` (HEAD atual). Ancestral comum: `git merge-base HEAD origin/main`.

### 3.1 Listar overlap

Arquivos alterados **em ambos** os lados desde o ancestral comum (comandos detalhados em [reference.md](reference.md)):

1. Commits novos do upstream:
   ```bash
   git log HEAD..origin/main --oneline
   ```
2. Interseção de paths tocados por `origin/main` e pela branch de personalizações desde o merge-base.

### 3.2 Tabela de veredito (por arquivo ou grupo coerente)

Para cada path (ou grupo, ex. `ribbon.rs` + `widgets.rs` do clipboard), registrar **um** veredito:

| Arquivo / grupo | Veredito | Motivo (1 linha) |
|-----------------|----------|------------------|
| `path` | **Manter o nosso** / **Pegar o do main** / **Combinar** | … |

Significado:

- **Manter o nosso** — personalização melhor ou intencional; resultado final deve preservar a intenção nossa (main só entra se for ortogonal e não diluir o recurso).
- **Pegar o do main** — upstream claramente melhor (bugfix, API correta, regressão nossa); nossa mudança no arquivo era acidental ou inferior.
- **Combinar** — trazer a melhoria do `main` **sobre a base nossa** (não o contrário): portar fix/API/campo novo do main sem descartar SQLite, locale PT, clipboard, caches, etc.

### 3.3 Critérios de decisão

Ao escolher o veredito, use nesta ordem:

1. **Intenção documentada** — Regra Fundamental (§1), playbook (§5), e `docs/personalizacoes.md` se existir.
2. **Testes de regressão das personalizações** — se o lado do main quebraria clipboard / fórmulas PT / SQLite / Esc-colar, não “Pegar o do main” sem plano de Combinar.
3. **Tamanho e clareza do diff** — diff pequeno e ortogonal no main favorece Combinar ou Pegar o do main; diff grande que apaga comportamento nosso favorece Manter o nosso ou Combinar cuidadoso.
4. **Ortogonalidade** — bump de versão, CI, lockfile, docs, deps sem tocar na lógica das personalizações → merge sem medo (veredito implícito: aceitar main nesses paths; ainda listar no overlap se ambos tocaram o mesmo arquivo).

### 3.4 Gate humano (PARAR)

Se o próximo `git merge` (incluindo auto-merge sem conflito de marcadores) for **tocar qualquer arquivo da lista de overlap** e esse arquivo/grupo **não tiver veredito registrado** na tabela desta sessão:

1. **PARAR** — não executar o merge.
2. Pedir confirmação ao usuário com a lista de overlap pendente e sugestão de veredito.
3. Só continuar após vereditos preenchidos (ou confirmação explícita do usuário para o conjunto).

Arquivos fora do overlap podem seguir o fluxo normal de merge.

### 3.5 Registrar o gate na sessão

Antes do merge, deixe explícito no raciocínio/resumo: merge-base, lista de overlap, tabela completa, e quais paths são ortogonais (CI/versão).

---

## 4. Fase 3: Executar o merge

Somente com a Fase 2 concluída:

```bash
git merge origin/main
```

Resolva conflitos com o playbook abaixo **e** respeitando os vereditos da Fase 2 (o playbook ilustra Combinar nos hotspots conhecidos; vereditos novos prevalecem quando houver conflito entre playbook genérico e tabela da sessão).

---

## 5. Fase 4: Playbook de Resolução de Conflitos Conhecidos

Se houver conflitos, siga este manual para os arquivos recorrentes (padrão típico: **Combinar**):

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

## 6. Fase 5: Pós-merge — restaurar intenção (“Manter o nosso”)

Após o merge (com ou sem conflitos marcados):

1. Para cada arquivo com veredito **Manter o nosso**, inspecione o resultado (`git diff HEAD1` / diff contra pré-merge / leitura do arquivo).
2. Se o resultado trouxe código do `main` **indesejado** (auto-merge “silencioso” ou resolução errada):
   - Restaurar a intenção: `git checkout --ours -- <path>` em seções/arquivos apropriados, **ou** reaplicar o patch/comportamento nosso sobre o que veio do main.
   - Em **Combinar**, não use `--ours` cego no arquivo inteiro — reaplique só o que for personalização e preserve o ganho do main.
3. Só então `git add` dos paths corrigidos e siga para os testes.

---

## 7. Fase 6: Bateria Obrigatória de Testes de Regressão

Após resolver conflitos / restaurar intenção e adicionar com `git add`, execute obrigatoriamente:

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

## 8. Fase 7: Finalização ou Cancelamento

- **Se todos os testes passarem com 100% de sucesso:**
  ```bash
  git commit -m "Merge origin/main into minhas-personalizacoes (descrição das melhorias integradas)"
  ```
- **Se algo quebrar e não puder ser resolvido de imediato:**
  ```bash
  git merge --abort
  ```
  *O repositório volta instantaneamente ao estado seguro pré-merge.* (O branch `backup-antes-do-merge` permanece como rede de segurança.)

---

## 9. Comportamento esperado no próximo `execute`

Ao executar esta skill, o agente deve:

1. Limpar/verificar working tree → backup → `git fetch`.
2. **Parar no gate:** listar overlap, montar tabela de vereditos, aplicar critérios; se faltar veredito em path que o merge tocaria → pedir confirmação humana.
3. Só então `git merge origin/main`.
4. Resolver conflitos com playbook + vereditos; pós-merge, auditar “Manter o nosso” e restaurar se o main vazou.
5. Rodar a bateria de testes; commit de merge se OK, senão `merge --abort`.
