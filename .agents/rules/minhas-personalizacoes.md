---
trigger: model_decision
description: Minhas-alteracoes
---

Quando houver uma nova versão
Salve primeiro suas modificações em um commit:
git add .
git commit -m "Minhas personalizações"

Depois, busque as novidades oficiais:
git fetch origin

Incorpore as atualizações da linha principal à sua ramificação:
git merge origin/main

Se houver alterações incompatíveis nos mesmos arquivos, o Git poderá solicitar que você resolva conflitos antes de concluir a atualização. Por isso, é importante revisar as mudanças e testar o programa após cada integração.
Importante: git fetch apenas busca as novidades; git merge integra essas novidades ao seu código. Nenhum dos dois garante que suas modificações serão compatíveis automaticamente. 4. E se você quiser personalizar bastante o Gridcraft?
Há duas estratégias:
Opção A — Manter uma ramificação própria
Recomendada para começar
Você mantém suas alterações em uma ramificação e integra periodicamente as atualizações oficiais. É mais simples para trabalhar sozinho no seu computador.

Opção B — Criar um fork no GitHub
Você cria uma cópia do repositório na sua conta, publica suas alterações nela e mantém o original como referência para atualizações. É interessante se quiser desenvolver seu próprio produto e manter um histórico remoto.
