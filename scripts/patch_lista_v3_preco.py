#!/usr/bin/env python3
"""Patch corpus LISTA TESTE V3: preço via ORCAMENTO[Linha] (refEq) + validação de [Preco_Unit].

O lookup pesado (`refEq` + fast path `n=1`) vive na coluna calculada `ORCAMENTO[Linha]` (planilha
coluna L). `ORCAMENTO[Preco_Unit]` (coluna J) permanece `INDEX(Preco_Venda|Preco_Revenda, Linha)`
com ramo filial/cliente (B1/B2); não usa `achou`/`chave`.

Este script reaplica `patch_lista_v3_linha.py` se ainda existir `_xlpm.achou` em Linha.
"""
from __future__ import annotations

import subprocess
import sys
import zipfile
from pathlib import Path

MARKER_ACHOU = "_xlpm.achou,"


def achou_count(xlsx: Path) -> int:
    total = 0
    with zipfile.ZipFile(xlsx) as z:
        for name in z.namelist():
            if name.endswith(".xml"):
                total += z.read(name).decode("utf-8", errors="ignore").count(MARKER_ACHOU)
    return total


def main() -> int:
    xlsx = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(r"c:\Users\Robson\source\grid-excel\LISTA TESTE V3.xlsx")
    if not xlsx.is_file():
        print(f"missing {xlsx}", file=sys.stderr)
        return 1

    n = achou_count(xlsx)
    if n == 0:
        print(f"{xlsx.name}: Linha already refEq; Preco_Unit unchanged (INDEX via Linha)")
        return 0

    linha_script = Path(__file__).with_name("patch_lista_v3_linha.py")
    print(f"{xlsx.name}: {n} achou marker(s) — running {linha_script.name}")
    return subprocess.call([sys.executable, str(linha_script), str(xlsx)])


if __name__ == "__main__":
    raise SystemExit(main())
