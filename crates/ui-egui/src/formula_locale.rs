//! UI formula syntax boundary. Engine commands, control/API callers and XLSX keep English.
use gridcraft_engine::formula::{self, FormulaLocale, ParseError};
use gridcraft_engine::model::Workbook;

pub(crate) fn canonical(
    text: &str,
    locale: FormulaLocale,
    wb: &Workbook,
    sheet: usize,
    cell: gridcraft_engine::core::CellRef,
) -> Result<String, ParseError> {
    // The editor hid the `_xludf.` prefixes of this cell's formula; confirming it unchanged keeps
    // the user-defined calls instead of binding them to the built-ins of the same name.
    if let Some(stored) = wb.sheet(sheet).and_then(|sh| sh.cell(cell)).filter(|c| c.formula.is_some()).map(|c| c.input_text())
        && has_udf_prefix(&stored)
        && text == display(&stored, locale, wb, sheet)
    {
        return Ok(stored);
    }
    if locale == FormulaLocale::En || wb.sheet(sheet).is_some_and(|sh| wb.styles.get(sh.style_id(cell)).num_fmt.as_str() == "@") {
        return Ok(text.to_string());
    }
    // Same formula detection as the engine's `input_to_cell` (no trimming: " =1" is text).
    let body = text;
    if body.starts_with('=')
        || ((body.starts_with('+') || body.starts_with('-')) && body.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic() || c == '('))
    {
        let known = |name: &str| wb.name(name, sheet).is_some() || wb.table(name).is_some();
        // Match the engine's existing convenience for missing closing parentheses.
        let mut body = body.to_string();
        let mut expr = formula::parse_input(&body, locale, known);
        for _ in 0..8 {
            if expr.is_ok() {
                break;
            }
            body.push(')');
            expr = formula::parse_input(&body, locale, known);
        }
        return Ok(format!("={}", formula::print(&expr?)));
    }
    // Decimal-comma number entry follows the same preference, without rewriting text or dates.
    let normalized = body.trim().replace(locale.decimal_separator(), ".");
    let number = normalized.strip_suffix('%').unwrap_or(&normalized);
    if number.parse::<f64>().is_ok_and(f64::is_finite) {
        return Ok(normalized);
    }
    Ok(text.to_string())
}

pub(crate) fn display(text: &str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> String {
    // English (and every language without a formula table) shows the stored text, `_xludf.`
    // prefixes aside, and skips a parse per frame.
    if locale == FormulaLocale::En {
        return hide_udf_prefix(text, locale);
    }
    let Some(body) = text.strip_prefix('=') else { return text.to_string() };
    let shown = formula::parse(body)
        .and_then(|expr| formula::print_input(&expr, locale, |name| wb.name(name, sheet).is_some() || wb.table(name).is_some()))
        .map(|body| format!("={body}"))
        .unwrap_or_else(|_| text.to_string());
    hide_udf_prefix(&shown, locale)
}

const UDF_PREFIX: &str = "_xludf.";

fn has_udf_prefix(text: &str) -> bool {
    text.to_ascii_lowercase().contains(UDF_PREFIX)
}

/// Excel shows a user-defined call such as `_xludf.EOMONTH()` as `=EOMONTH()`. Only function
/// names lose the prefix: text literals and names keep it.
fn hide_udf_prefix(text: &str, locale: FormulaLocale) -> String {
    let Some(body) = text.strip_prefix('=').filter(|b| has_udf_prefix(b)) else { return text.to_string() };
    let Ok(tokens) = formula::lexer::tokenize_locale(body, locale) else { return text.to_string() };
    let mut out = String::from("=");
    let mut from = 0;
    for t in tokens {
        if matches!(t.tok, formula::lexer::Tok::Func(_))
            && body.get(t.start..t.start + UDF_PREFIX.len()).is_some_and(|p| p.eq_ignore_ascii_case(UDF_PREFIX))
        {
            out.push_str(body.get(from..t.start).unwrap_or(""));
            from = t.start + UDF_PREFIX.len();
        }
    }
    out.push_str(body.get(from..).unwrap_or(""));
    out
}

// Descriptions/signatures for the supported Spanish function table; untranslated functions
// retain their English help. Parameter punctuation follows the same locale as entry.
const SPANISH_HELP: &[(&str, &str, &str)] = &[
    ("SUM", "SUMA(número1; [número2]; ...)", "Suma los números."),
    ("AVERAGE", "PROMEDIO(número1; [número2]; ...)", "Devuelve el promedio de los números."),
    ("IF", "SI(prueba_lógica; valor_si_verdadero; [valor_si_falso])", "Devuelve un valor según una condición."),
    ("COUNT", "CONTAR(valor1; [valor2]; ...)", "Cuenta las celdas que contienen números."),
    ("SUMIF", "SUMAR.SI(rango; criterio; [rango_suma])", "Suma las celdas que cumplen un criterio."),
    ("COUNTIF", "CONTAR.SI(rango; criterio)", "Cuenta las celdas que cumplen un criterio."),
    ("VLOOKUP", "BUSCARV(valor_buscado; matriz; indicador_columnas; [ordenado])", "Busca un valor en la primera columna de una tabla."),
    ("TODAY", "HOY()", "Devuelve la fecha actual."),
    ("TRUE", "VERDADERO()", "Devuelve el valor lógico VERDADERO."),
    ("FALSE", "FALSO()", "Devuelve el valor lógico FALSO."),
];

const PORTUGUESE_HELP: &[(&str, &str, &str)] = &[
    ("SUM", "SOMA(número1; [número2]; ...)", "Soma todos os números em um intervalo de células."),
    ("AVERAGE", "MÉDIA(número1; [número2]; ...)", "Retorna a média aritmética dos argumentos."),
    (
        "IF",
        "SE(teste_lógico; valor_se_verdadeiro; [valor_se_falso])",
        "Verifica se uma condição foi satisfeita e retorna um valor se for VERDADEIRO e outro se for FALSO.",
    ),
    (
        "IFS",
        "SES(teste_lógico1; valor_se_verdadeiro1; ...)",
        "Verifica se uma ou mais condições foram satisfeitas e retorna o valor correspondente à primeira condição VERDADEIRA.",
    ),
    (
        "IFERROR",
        "SEERRO(valor; valor_se_erro)",
        "Retorna um valor especificado se a fórmula resultar em erro; caso contrário, retorna o resultado da fórmula.",
    ),
    ("COUNT", "CONT.NÚM(valor1; [valor2]; ...)", "Calcula o número de células em um intervalo que contêm números."),
    ("COUNTA", "CONT.VALORES(valor1; [valor2]; ...)", "Calcula o número de células não vazias em um intervalo."),
    ("COUNTIF", "CONT.SE(intervalo; critérios)", "Calcula o número de células que atendem aos critérios determinados."),
    ("COUNTIFS", "CONT.SES(intervalo_critérios1; critérios1; ...)", "Conta o número de vezes que várias condições são atendidas."),
    ("SUMIF", "SOMASE(intervalo; critérios; [intervalo_soma])", "Adiciona as células especificadas por um determinado critério ou condição."),
    (
        "SUMIFS",
        "SOMASES(intervalo_soma; intervalo_critérios1; critérios1; ...)",
        "Soma as células em um intervalo que atendem a múltiplos critérios.",
    ),
    (
        "VLOOKUP",
        "PROCV(valor_procurado; matriz_tabela; núm_índice_coluna; [procurar_intervalo])",
        "Procura um valor na primeira coluna à esquerda de uma tabela e retorna um valor na mesma linha de outra coluna especificada.",
    ),
    (
        "XLOOKUP",
        "PROCX(pesquisa_valor; matriz_pesquisa; matriz_retorno; [se_não_encontrada]; [modo_correspondência]; [modo_pesquisa])",
        "Pesquisa um intervalo ou matriz e retorna o item correspondente da matriz de retorno.",
    ),
    (
        "MATCH",
        "CORRESP(valor_procurado; matriz_procurada; [tipo_correspondência])",
        "Retorna a posição relativa de um item em uma matriz correspondente a um valor especificado.",
    ),
    (
        "XMATCH",
        "CORRESPX(pesquisa_valor; matriz_pesquisa; [modo_correspondência]; [modo_pesquisa])",
        "Retorna a posição relativa de um item em uma matriz ou intervalo de células.",
    ),
    (
        "INDEX",
        "ÍNDICE(matriz; núm_linha; [núm_coluna])",
        "Retorna o valor de um elemento em uma tabela ou matriz, selecionado pelos índices de linha e coluna.",
    ),
    (
        "CHOOSE",
        "ESCOLHER(núm_índice; valor1; [valor2]; ...)",
        "Escolhe um valor ou uma ação a ser executada a partir de uma lista de valores com base em um número de índice.",
    ),
    (
        "OFFSET",
        "DESLOC(ref; linhas; cols; [altura]; [largura])",
        "Retorna uma referência a um intervalo que é um número especificado de linhas e colunas de uma célula ou intervalo.",
    ),
    ("AND", "E(lógico1; [lógico2]; ...)", "Verifica se todos os argumentos são VERDADEIRO e retorna VERDADEIRO se todos forem verdadeiros."),
    ("OR", "OU(lógico1; [lógico2]; ...)", "Verifica se qualquer um dos argumentos é VERDADEIRO e retorna VERDADEIRO ou FALSO."),
    ("NOT", "NÃO(lógico)", "Inverte o valor lógico do argumento."),
    ("TODAY", "HOJE()", "Retorna a data atual formatada como data."),
    ("NOW", "AGORA()", "Retorna a data e hora atuais formatadas como data e hora."),
    ("ROUND", "ARRED(número; núm_dígitos)", "Arredonda um número até uma quantidade especificada de dígitos."),
    ("CONCATENATE", "CONCATENAR(texto1; [texto2]; ...)", "Agrupa várias sequências de caracteres de texto em uma única sequência."),
    ("LEFT", "ESQUERDA(texto; [núm_caract])", "Retorna os primeiros caracteres à esquerda de uma sequência de texto."),
    ("RIGHT", "DIREITA(texto; [núm_caract])", "Retorna os últimos caracteres à direita de uma sequência de texto."),
    (
        "MID",
        "EXT.TEXTO(texto; núm_inicial; núm_caract)",
        "Retorna um número específico de caracteres de uma sequência de texto, a partir da posição especificada.",
    ),
    ("LEN", "NÚM.CARACT(texto)", "Retorna o número de caracteres em uma sequência de texto."),
    ("UNIQUE", "ÚNICO(matriz; [por_coluna]; [exatamente_uma_vez])", "Retorna uma lista de valores exclusivos em uma lista ou intervalo."),
    ("FILTER", "FILTRO(matriz; incluir; [se_vazia])", "Filtra um intervalo de dados com base nos critérios definidos."),
    ("SORT", "CLASSIFICAR(matriz; [índice_ordenação]; [ordem_classificação]; [por_coluna])", "Classifica o conteúdo de um intervalo ou matriz."),
    ("TRUE", "VERDADEIRO()", "Retorna o valor lógico VERDADEIRO."),
    ("FALSE", "FALSO()", "Retorna o valor lógico FALSO."),
];

pub(crate) fn signature(name: &str, locale: FormulaLocale) -> Option<String> {
    let canonical = locale.canonical_name(name);
    if locale == FormulaLocale::Es
        && let Some((_, signature, _)) = SPANISH_HELP.iter().find(|(name, ..)| *name == canonical)
    {
        return Some((*signature).into());
    }
    if locale == FormulaLocale::Pt
        && let Some((_, signature, _)) = PORTUGUESE_HELP.iter().find(|(name, ..)| *name == canonical)
    {
        return Some((*signature).into());
    }
    crate::formula_bar::function_signature(canonical).map(|s| s.replace(',', &locale.list_separator().to_string()))
}

pub(crate) fn description(name: &str, locale: FormulaLocale) -> Option<String> {
    let canonical = locale.canonical_name(name);
    if locale == FormulaLocale::Es
        && let Some((_, _, description)) = SPANISH_HELP.iter().find(|(name, ..)| *name == canonical)
    {
        return Some((*description).into());
    }
    if locale == FormulaLocale::Pt
        && let Some((_, _, description)) = PORTUGUESE_HELP.iter().find(|(name, ..)| *name == canonical)
    {
        return Some((*description).into());
    }
    crate::formula_bar::function_description(canonical)
}

/// Avoid suggesting an alias that would call a workbook/table name instead of the function.
pub(crate) fn completion_name<'a>(name: &'a str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> &'a str {
    let alias = locale.function_name(name);
    if wb.name(alias, sheet).is_some() || wb.table(alias).is_some() { name } else { alias }
}

/// Engine-generated edit templates use canonical syntax. The only incomplete template is
/// the bare function opening emitted by Insert Function; typed user input never enters here.
pub(crate) fn display_template(text: &str, locale: FormulaLocale, wb: &Workbook, sheet: usize) -> String {
    if let Some(name) = text.strip_prefix('=').and_then(|s| s.strip_suffix('('))
        && !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
    {
        return format!("={}(", completion_name(name, locale, wb, sheet));
    }
    display(text, locale, wb, sheet)
}
