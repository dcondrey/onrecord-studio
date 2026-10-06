# calamine 0.36.1, patched

Upstream: https://github.com/tafia/calamine at tag `v0.36.1`. Local fork with the change on branch
`fast-cell-reader` (commit 55e8539) in `/Volumes/A/calamine-fork`; this directory is that fork packaged with
`cargo package`.

The change: `XlsxCellReader::next_cell` reads text, `<row>`, `</row>` and `<c>` elements whose
children are only `<v>`/`<f>` with plain text straight from the reader's buffer, whenever the XML
reader's last event was an end tag. Anything else is left unconsumed for quick-xml. The sheet
reader allows unmatched end tags, because a `<row>` read by the fast path can be closed by
quick-xml.

Evidence: calamine's own tests pass; every sheet of the 64 XLSX workbooks in calamine's
`tests/` (2.9M cells) gives identical cells, values and errors to the released 0.36.1;
`next_cell` over the 118,550-row arrests workbook went from 0.45 s to 0.27 s.

Remove this directory and the `[patch.crates-io]` entry when the change is released upstream.
