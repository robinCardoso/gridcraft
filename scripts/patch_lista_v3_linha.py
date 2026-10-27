#!/usr/bin/env python3
"""Patch ORCAMENTO[Linha] in LISTA TESTE V3.xlsx (refEq + n=1 fast path)."""
from __future__ import annotations

import shutil
import sys
import zipfile
from pathlib import Path

MARKER_OLD = "_xlfn.LET(_xlpm.achou,"

NEW_INNER = (
    "_xlfn.LET(_xlpm.refEq,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM(LISTA_PRECO[Referencia]&\"\"),\" \",\"\"),\".\",\"\"),\"-\",\"\"))=_xlpm.codigo,_xlpm.n,SUM(--_xlpm.refEq),IF(_xlpm.n=0,0,_xlfn.LET(_xlpm.cadF,UPPER(SUBSTITUTE(SUBSTITUTE(CAD_FORNECEDOR[Filiais_atendidas]&\"\",\" \",\"\"),\",\",\";\")),_xlpm.cadOk,(_xlpm.cadF=\"\")+(_xlpm.cadF=\"TODAS\")+ISNUMBER(SEARCH(\";\"&_xlpm.filial&\";\",\";\"&_xlpm.cadF&\";\")),_xlpm.naoServem,_xlfn._xlws.FILTER(CAD_FORNECEDOR[Razao_social],_xlpm.cadOk=0,\"§\"),IF(_xlpm.n=1,_xlfn.LET(_xlpm.ri,MATCH(TRUE,UPPER(SUBSTITUTE(SUBSTITUTE(SUBSTITUTE(TRIM(LISTA_PRECO[Referencia]&\"\"),\" \",\"\"),\".\",\"\"),\"-\",\"\"))=_xlpm.codigo,0),_xlpm.at,ISNA(MATCH(TRIM(INDEX(LISTA_PRECO[Fornecedor],_xlpm.ri)),_xlpm.naoServem,0)),_xlpm.pn,IF(_xlpm.filial=_xlpm.cliente,INDEX(LISTA_PRECO[Preco_Venda],_xlpm.ri),INDEX(LISTA_PRECO[Preco_Revenda],_xlpm.ri)),IF(OR(NOT(_xlpm.at),NOT(ISNUMBER(_xlpm.pn)),_xlpm.pn<=0),-1,_xlpm.ri)),_xlfn.LET(_xlpm.atende,ISNA(MATCH(TRIM(LISTA_PRECO[Fornecedor]),_xlpm.naoServem,0)),_xlpm.pc,IF(_xlpm.filial=_xlpm.cliente,LISTA_PRECO[Preco_Venda],LISTA_PRECO[Preco_Revenda]),_xlpm.pn,IF(ISNUMBER(_xlpm.pc),_xlpm.pc,0),_xlpm.ok,_xlpm.refEq*_xlpm.atende*(_xlpm.pn>0),IF(SUM(_xlpm.ok)=0,-1,_xlfn.XMATCH(1,_xlpm.ok*(_xlpm.pn=MIN(_xlfn._xlws.FILTER(_xlpm.pn,_xlpm.ok))))))))))"
)

PATCH_FILES = (
    "xl/tables/table1.xml",
    "xl/worksheets/sheet1.xml",
)


def xml_escape_formula(s: str) -> str:
    # Order matters: & first, then < (OR(...,pn<=0) breaks table XML otherwise).
    return s.replace("&", "&amp;").replace("<", "&lt;")


def balanced_span(text: str, start: int) -> int:
    depth = 0
    for i in range(start, len(text)):
        ch = text[i]
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i + 1
    raise ValueError("unbalanced parens")


def patch_text(text: str) -> tuple[str, int]:
    count = 0
    while True:
        pos = text.find(MARKER_OLD)
        if pos < 0:
            break
        end = balanced_span(text, pos)
        text = text[:pos] + xml_escape_formula(NEW_INNER) + text[end:]
        count += 1
    return text, count


def patch_bytes(data: bytes) -> tuple[bytes, int]:
    text = data.decode("utf-8")
    patched, count = patch_text(text)
    if count == 0:
        return data, 0
    return patched.encode("utf-8"), count


def main() -> int:
    xlsx = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx")
    if not xlsx.is_file():
        print(f"missing {xlsx}", file=sys.stderr)
        return 1

    depth = 0
    for ch in NEW_INNER:
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
    if depth != 0:
        print(f"NEW_INNER unbalanced: {depth}", file=sys.stderr)
        return 1

    tmp = xlsx.with_suffix(".patched.xlsx")
    total = 0
    with zipfile.ZipFile(xlsx, "r") as zin, zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED) as zout:
        for info in zin.infolist():
            raw = zin.read(info.filename)
            if info.filename in PATCH_FILES:
                raw, n = patch_bytes(raw)
                total += n
            zout.writestr(info, raw)
    if total == 0:
        print("no replacements — already patched or formula mismatch", file=sys.stderr)
        tmp.unlink(missing_ok=True)
        return 2
    shutil.move(tmp, xlsx)
    print(f"patched {xlsx.name}: {total} formula occurrence(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
