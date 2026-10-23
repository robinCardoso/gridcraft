# Referência — Gate de overlap e vereditos

Complemento de [SKILL.md](SKILL.md). Usar na **Fase 2**, após `git fetch` e **antes** de `git merge`.

## Variáveis

- `OURS` = branch de personalizações (ex.: `HEAD` / `minhas-personalizacoes`)
- `THEIRS` = upstream (ex.: `origin/main`)
- `BASE` = ancestral comum

```bash
BASE=$(git merge-base HEAD origin/main)
```

## Listar commits do upstream ainda não integrados

```bash
git log HEAD..origin/main --oneline
```

## Listar arquivos tocados só de um lado

```bash
# Alterados no main desde o ancestral
git diff --name-only $BASE..origin/main

# Alterados nas personalizações desde o ancestral
git diff --name-only $BASE..HEAD
```

## Overlap (interseção)

PowerShell (Windows):

```powershell
$base = git merge-base HEAD origin/main
$main = git diff --name-only $base..origin/main
$ours = git diff --name-only $base..HEAD
Compare-Object $main $ours -IncludeEqual -ExcludeDifferent | ForEach-Object { $_.InputObject }
```

Bash / Git Bash:

```bash
comm -12 \
  <(git diff --name-only "$BASE"..origin/main | sort -u) \
  <(git diff --name-only "$BASE"..HEAD | sort -u)
```

## Diff focado por arquivo do overlap

```bash
git diff $BASE..HEAD -- path/to/file      # o nosso desde o ancestral
git diff $BASE..origin/main -- path/to/file  # o main desde o ancestral
```

## Modelo de tabela de vereditos (copiar na sessão)

| Arquivo / grupo | Veredito | Motivo |
|-----------------|----------|--------|
| `crates/...` | Manter o nosso | Intenção SQLite / clipboard / locale PT |
| `crates/...` | Pegar o do main | Bugfix upstream; nossa mudança acidental |
| `crates/...` | Combinar | Trazer API/campo do main sobre base nossa |
| `Cargo.lock` (se só bump ortogonal) | Pegar o do main / Combinar | Versão/CI ortogonal |

Vereditos válidos: **Manter o nosso** | **Pegar o do main** | **Combinar**.

## Gate humano

Se `git merge` for alterar um path da interseção **sem** linha na tabela → **PARAR** e pedir confirmação ao usuário.

## Pós-merge: “Manter o nosso” contaminado

```bash
# Comparar com o estado pré-merge (backup ou primeiro pai durante merge)
git show backup-antes-do-merge:path/to/file
git checkout --ours -- path/to/file   # só quando o veredito for Manter o nosso no arquivo inteiro
# Preferível em Combinar: editar manualmente ou reaplicar patch seletivo
```

## Checklist de personalizações (obrigatório)

Arquivo: [`docs/personalizacoes.md`](../../../docs/personalizacoes.md).

Na Fase 2, **antes** da tabela de vereditos:

1. Abrir o doc.
2. Para cada path do overlap, grep/diff contra as âncoras da tabela “Regressões recentes” (`needs_formula_eval`, `apply_job`/`revision`, `resolve_struct`, `invalidate_graph`, `recalc_cascade`, clipboard consume, `coop`/yield, `truncate(1_000)` / Automatic).
3. Se houver match → veredito **Manter o nosso** ou **Combinar**, Motivo com o `#` do item.
4. No resumo da sessão: “`personalizacoes.md` consultado; itens tocados: … / nenhum”.

O gate de overlap sozinho **não** enumera esses bugs — só paths. A checklist é o que impede regressão silenciosa por auto-merge.
