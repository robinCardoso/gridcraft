#!/usr/bin/env python3
"""Split COTACAO!L1: M1 caches E1-only ref match (R{row}|M{n}|NL); L1 uses M1 on B1/B2 edits."""
from __future__ import annotations

import re
import shutil
import sys
import zipfile
from pathlib import Path

# Depends only on $E$1 and LISTA_PRECO[Referencia] (ColEq peelable).
M1_FORMULA = (
    '_xlfn.LET(_xlpm.codigo,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM($E$1&"")," ",""),".",""),"-","")),'
    'IF(_xlpm.codigo="","",'
    '_xlfn.LET(_xlpm.refEq,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM(LISTA_PRECO[Referencia]&"")," ",""),".",""),"-",""))=_xlpm.codigo,'
    '_xlpm.n,SUM(--_xlpm.refEq),IF(_xlpm.n=0,"NL",IF(_xlpm.n=1,MATCH(TRUE,_xlpm.refEq,0),"M"&TEXT(_xlpm.n,"0"))))))'
)

L1_FORMULA = (
    '_xlfn.LET(_xlpm.filial,UPPER(TRIM($B$1)),_xlpm.cliente,UPPER(TRIM(IF($B$2="",$B$1,$B$2))),'
    'IF($M$1="","",IF($M$1="NL","NL",'
    'IF(ISNUMBER($M$1),'
    '_xlfn.LET(_xlpm.ri,$M$1,'
    '_xlpm.cadF,UPPER(SUBSTITUTE(SUBSTITUTE(CAD_FORNECEDOR[Filiais_atendidas]&""," ",""),",",";")),'
    '_xlpm.cadOk,(_xlpm.cadF="")+(_xlpm.cadF="TODAS")+ISNUMBER(SEARCH(";"&_xlpm.filial&";",";"&_xlpm.cadF&";")),'
    '_xlpm.naoServem,_xlfn._xlws.FILTER(CAD_FORNECEDOR[Razao_social],_xlpm.cadOk=0,"§"),'
    '_xlpm.at,ISNA(MATCH(TRIM(INDEX(LISTA_PRECO[Fornecedor],_xlpm.ri)),_xlpm.naoServem,0)),'
    'IF(NOT(_xlpm.at),"NF",TEXT(_xlpm.ri,"0"))),'
    '_xlfn.LET(_xlpm.codigo,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM($E$1&"")," ",""),".",""),"-","")),'
    '_xlpm.refEq,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM(LISTA_PRECO[Referencia]&"")," ",""),".",""),"-",""))=_xlpm.codigo,'
    '_xlpm.cadF,UPPER(SUBSTITUTE(SUBSTITUTE(CAD_FORNECEDOR[Filiais_atendidas]&""," ",""),",",";")),'
    '_xlpm.cadOk,(_xlpm.cadF="")+(_xlpm.cadF="TODAS")+ISNUMBER(SEARCH(";"&_xlpm.filial&";",";"&_xlpm.cadF&";")),'
    '_xlpm.naoServem,_xlfn._xlws.FILTER(CAD_FORNECEDOR[Razao_social],_xlpm.cadOk=0,"§"),'
    '_xlpm.atende,ISNA(MATCH(TRIM(LISTA_PRECO[Fornecedor]),_xlpm.naoServem,0)),'
    '_xlpm.ok,_xlpm.refEq*_xlpm.atende,IF(SUM(_xlpm.ok)=0,"NF",'
    '_xlfn.LET(_xlpm.pc,IF(_xlpm.filial=_xlpm.cliente,LISTA_PRECO[Preco_Venda],LISTA_PRECO[Preco_Revenda]),'
    '_xlpm.pn,IF(ISNUMBER(_xlpm.pc),_xlpm.pc,0),'
    '_xlfn.TEXTJOIN(";",TRUE,_xlfn.SORTBY(_xlfn._xlws.FILTER(_xlfn.SEQUENCE(ROWS(LISTA_PRECO[Referencia])),_xlpm.ok),'
    '_xlfn._xlws.FILTER(IF(_xlpm.pn>0,_xlpm.pn,9E+99),_xlpm.ok),1)))))))))'
)


def xml_escape_formula(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;")


def row_number(ref: str) -> str:
    m = re.match(r"^[A-Z]+(\d+)$", ref)
    if not m:
        raise ValueError(f"bad cell ref {ref}")
    return m.group(1)


def set_cell_formula(sheet_xml: str, ref: str, formula: str) -> str:
    esc = xml_escape_formula(formula)
    rn = row_number(ref)
    ns = r"(?:[\w]+:)?"
    row = re.search(rf"<{ns}row r=\"{rn}\"[^>]*>", sheet_xml)
    if not row:
        raise ValueError(f"row r={rn} for {ref} not found")
    row_start = row.start()
    row_close = re.search(rf"</{ns}row>", sheet_xml[row_start:])
    if not row_close:
        raise ValueError("row end not found")
    row_end = row_start + row_close.end()
    row_body = sheet_xml[row_start:row_end]

    cell_pat = rf"(<{ns}c r=\"{ref}\"[^>]*>)(.*?)(</{ns}c>)"
    cell_m = re.search(cell_pat, row_body, re.DOTALL)
    prefix_m = re.search(r"<([\w]+:)c r=", row_body)
    cp = prefix_m.group(1) if prefix_m else ""
    new_cell = f"<{cp}c r=\"{ref}\"><{cp}f>{esc}</{cp}f></{cp}c>"
    row_close_tag = f"</{cp}row>" if cp else "</row>"
    if cell_m:
        row_body = row_body[: cell_m.start()] + new_cell + row_body[cell_m.end() :]
    else:
        row_body = row_body.replace(row_close_tag, new_cell + row_close_tag, 1)

    return sheet_xml[:row_start] + row_body + sheet_xml[row_end:]


def patch_sheet(sheet_xml: str) -> str:
    if "ISNUMBER($M$1)" in sheet_xml and '"R"&MATCH' not in sheet_xml:
        return sheet_xml
    sheet_xml = set_cell_formula(sheet_xml, "M1", M1_FORMULA)
    sheet_xml = set_cell_formula(sheet_xml, "L1", L1_FORMULA)
    return sheet_xml


def main() -> int:
    xlsx = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx")
    if not xlsx.is_file():
        print(f"missing {xlsx}", file=sys.stderr)
        return 1

    backup = xlsx.with_name("LISTA TESTE V3.pre-l1-split.xlsx")
    if not backup.is_file():
        shutil.copy2(xlsx, backup)
        print(f"backup -> {backup}")

    tmp = xlsx.with_suffix(".patched.xlsx")
    with zipfile.ZipFile(xlsx, "r") as zin, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
        for info in zin.infolist():
            raw = zin.read(info.filename)
            if info.filename == "xl/worksheets/sheet1.xml":
                text = raw.decode("utf-8")
                text = patch_sheet(text)
                raw = text.encode("utf-8")
            zout.writestr(info, raw)
    tmp.replace(xlsx)
    print(f"patched {xlsx} (M1 E1-cache + L1 split)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
