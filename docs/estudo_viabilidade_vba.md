# Estudo de Viabilidade Técnica: Implementação de VBA no GridCraft

Este documento apresenta uma análise técnica completa sobre o que seria necessário para implementar a execução de macros **VBA (Visual Basic for Applications)** no sistema GridCraft, os prazos estimados, os riscos arquiteturais e o impacto de **custos de consumo de IA**.

---

## 1. O que é o VBA e por que ele é diferente de fórmulas normais?

Diferente de fórmulas de planilha (como `SOMA`, `PROCV`, `SE`), que são puramente matemáticas e reativas, o **VBA é uma linguagem de programação procedural completa dos anos 90**, acoplada a um modelo de objetos imenso da Microsoft.

Quando um usuário clica em um botão como *"Limpa Pedido"* em um arquivo `.xlsm`, acontecem dezenas de coisas nos bastidores do Excel:
1. O Excel lê um arquivo binário comprimido proprietário chamado `vbaProject.bin`.
2. Um interpretador proprietário da Microsoft analisa o código Basic compilado/p-code.
3. O código interage com a memória viva do Excel através de uma interface COM (Component Object Model), manipulando objetos como `ActiveSheet`, `Range("A6:C50").ClearContents`, etc.

---

## 2. O que seria necessário construir para suportar VBA no GridCraft?

Para que o GridCraft execute macros VBA nativamente em Rust, seria necessário desenvolver do zero **5 camadas de software altamente complexas**:

1. **Leitor e Descompressor do Formato Binário (`MS-OVBA`)**:
   - O código VBA não fica em texto simples dentro do `.xlsm`. Ele é armazenado em um container OLE2/CFBF estruturado em streams comprimidos pelo algoritmo RLE da Microsoft.
   - Seria preciso criar um descompactador binário completo em Rust para extrair o código-fonte legível.

2. **Parser, AST e Interpretador da Linguagem Basic (`MS-VBAL`)**:
   - Implementar a gramática completa do Visual Basic 6: variáveis `Dim`, loops `For/Next`, `Do While`, condicionais `If/Else/End If`, tratamento de erros `On Error Resume Next/GoTo`, procedimentos `Sub` e `Function`, escopos de módulo, etc.

3. **Modelo de Objetos do Excel (Excel Object Model)**:
   - O código VBA não faz nada sozinho; ele depende de manipular a planilha.
   - Seria preciso programar centenas de objetos e métodos em Rust, como:
     - `Application` (`ScreenUpdating`, `Calculation`, `DisplayAlerts`, `ActiveWorkbook`)
     - `Worksheets("...")` (`Range`, `Cells`, `Rows`, `Columns`)
     - `Range` (`Value`, `Formula`, `ClearContents`, `Copy`, `PasteSpecial`, `Offset`, `Interior.Color`)
     - Métodos de seleção e cursor (`Select`, `Activate`)

4. **Renderização e Interação de Controles de Formulário (VML / Form Controls)**:
   - Ler arquivos `vmlDrawing.vml` e `ctrlProp.xml`.
   - Renderizar o botão gráfico na interface egui (`Limpa Pedido`).
   - Mapear o clique do mouse no botão para a subrotina VBA correspondente.

5. **Sandbox de Segurança**:
   - O VBA tradicional tem acesso a arquivos do Windows, registro e comandos de sistema (`Shell`, `CreateObject("Scripting.FileSystemObject")`).
   - Sem uma camada rígida de segurança, rodar macros desconhecidas pode comprometer o computador do usuário.

---

## 3. Estimativa de Tempo e Complexidade

| Escopo | Complexidade | Tempo Humano Especializado | Consumo de IA |
| :--- | :--- | :--- | :--- |
| **Suporte Completo a VBA** (todas as funções, objetos e formulários) | **Extrema** | 6 a 12 meses de engenharia dedicada | Centenas de sessões / Milhões de tokens (Risco alto de alucinações e loops de depuração) |
| **Suporte Básico a VBA** (interpretar apenas um subconjunto restrito como `ClearContents`, loops simples e atribuição de células) | **Alta** | 4 a 8 semanas | ~30 a 50 sessões longas de IA |
| **Ponte de Automação Alternativa** (Scripts em JavaScript/TypeScript ou Python nativo) | **Média** | 1 a 2 semanas | ~10 a 15 sessões de IA |
| **Mapeamento Cirúrgico Nativo** (Ação específica do botão "Limpa Pedido" em Rust) | **Baixa** | Poucas horas | 1 a 2 sessões de IA |

> [!NOTE]
> **Contexto da Indústria:**  
> Nem mesmo gigantes como o **Google Planilhas** ou o **Apple Numbers** suportam execução de macros VBA do Excel. O Google Planilhas optou pelo **Google Apps Script** (JavaScript), e o LibreOffice levou mais de 15 anos para conseguir rodar parte das macros VBA mais populares.

---

## 4. Análise de Custo de IA (Tokens e Retorno sobre o Investimento)

Desenvolver um interpretador de linguagem e um modelo de objetos completo via assistente de IA apresenta um **custo desproporcional** pelos seguintes motivos:

1. **Volume massivo de código contextual:**
   - Para que a IA programe um interpretador sem quebrar as regras de tipagem do Rust, ela precisa ler e manter centenas de arquivos de contexto, consumindo milhares de tokens a cada mensagem.
2. **Ciclos longos de depuração (Debugging Loops):**
   - Interpretadores sofrem com casos de borda infinitos (ex: tipos incompatíveis de variáveis em tempo de execução, chamadas recursivas, precedência de operadores). Cada bug consome rodadas repetidas de compilação e teste.
3. **Alto Risco de Quebra Arquitetural:**
   - O GridCraft segue uma arquitetura modular rígida em 7 camadas (`AGENTS.md`). Adicionar um motor VBA exige tocar em camadas de motor (`engine`), interface (`ui-egui`) e modelo (`model`), aumentando o risco de regressões em funções que já funcionam bem.

---

## 5. Estratégias Recomendadas para Economizar Custos de IA

Para evitar desperdício de créditos e tempo de IA, recomendamos escolher entre as seguintes abordagens:

### Opção A: Manter o Comportamento Atual (Custo Zero de IA) — *Recomendado*
- O GridCraft já faz o que o Google Planilhas faz: **preserva o código VBA intacto**.
- Se você editar e salvar o arquivo no GridCraft, nada do VBA é corrompido ou perdido. Quando o arquivo for reaberto no Microsoft Excel, as macros estarão 100% funcionais lá.
- **Custo:** R$ 0 / 0 tokens.

### Opção B: Melhoria Visual e Desativação do Popup Chato (Custo Mínimo de IA)
- O único incômodo real hoje é o popup modal que surge ao abrir o arquivo exigindo clicar em "OK".
- Podemos apenas:
  1. Suprimir o popup modal bloqueante.
  2. Manter apenas a faixa amarela sutil no topo (traduzida para português).
- **Tempo:** 10 minutos.
- **Custo:** Praticamente nulo (~1 turno de IA).

### Opção C: Mapeamento da Ação Específica ("Limpar Pedido") sem VBA
- Se a sua necessidade diária nessa planilha for apenas limpar o pedido:
  - Em vez de criar um interpretador de VBA, criamos um botão nativo ou atalho no menu que limpa as células do pedido diretamente em Rust.
- **Tempo:** 1 a 2 horas.
- **Custo:** Muito baixo (~2 turnos de IA).

---

## 6. Conclusão

Implementar um motor VBA completo no GridCraft **não é recomendado**, pois exigiria um investimento massivo de tempo e tokens de IA para recriar uma tecnologia legada da Microsoft dos anos 90 que a própria indústria já substituiu.

A melhor decisão técnica e financeira é **manter a preservação segura do arquivo** e apenas otimizar as mensagens da interface para que fiquem discretas e em português.
