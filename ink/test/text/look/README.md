# Reference cells for the cell-parity test

`parity.test.tsx` draws `<screen>_<w>x<h>.json` (the view data the host sends: chrome, screen, focus) through the whole Ink App and compares every cell with `<screen>_<w>x<h>.cells.json` (the reference buffer for the same host state at truecolor, icons off: `[symbol, fg, bg, modifier bits]`).

The cell files were recorded once from the earlier renderer, before it was retired, and are frozen as Ink's own goldens. The scratch generator that wrote them is gone with that renderer. To change a look, change Ink and read the diff of the golden it moves; do not regenerate the cells from another renderer.
