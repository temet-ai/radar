# Ink's goldens

The files here, `../fixtures/motion`, `../fixtures/wrap` and `../input/intents.json` are Ink's own goldens:
frozen, committed, and owned by Ink. They were first written from the the reference renderer (the styled cells of every
surface at 80x24 and 160x40, the motion engine's frames, the reference's word wrap, the adapter's intents). Nothing in Rust
writes them any more, and no Rust test compares them.

To move one, rewrite it from Ink and read the diff before committing:

```bash
cd ink
INK_UPDATE_GOLDENS=1 npx vitest run test/<dir-or-file>   # the test that reads the golden rewrites it from Ink's output
git diff -- test/cells test/fixtures test/input
```

| Golden | Rewritten by (with `INK_UPDATE_GOLDENS=1`) |
|---|---|
| `cells/highlight_typing-filter-*` | `test/filter/filter.test.tsx` |
| `cells/highlight_detail-sections-160x40` | `test/nodes/section-cells.test.tsx` |
| `cells/highlight_folded-row-*` | `test/nodes/tree-native-cells.test.tsx` |
| `cells/detail_{gitlab,jira,slack}-*` | `test/nodes/detail-native-cells.test.tsx` |
| `cells/highlight_hovered-row-*` | `test/app/hover.test.tsx` |
| `cells/highlight_selection-*` | `test/app/selection-cells.test.tsx` |
| `cells/highlight_menu-*` | `test/app/scrim-cells.test.tsx` |
| `cells/github_github*` | `test/chips/chip-ground-cells.test.tsx` |
| `fixtures/motion/*.json` | `test/motion/fixtures.test.ts` |
| `fixtures/wrap/golden.json` | `test/wrap/wrap.test.ts` |
| `input/intents.json` | `test/input/intents.test.ts` |

The other scenes in `cells/` (the screens, settings sections and item pages) are drawn in Storybook beside Ink's own
picture; no test reads them. A scene a test does not rewrite keeps the picture it was frozen with.

Icon packs (nerd, unicode, emoji, off) and colour slots are proved by `test/text/look/icon-packs.test.ts` and
`tiers.test.tsx` on Ink's own output, not by these files.

## Scene format (`format: 1`)

```text
{ "format": 1, "scene": "<name>", "width": W, "height": H,
  "canvas": { "fg": "text.primary", "bg": "bg.base" },
  "colours": { "<slot>": { "hex": "#rrggbb", "ink": ["accent"] } },
  "view": { "screen": ScreenView|null, "menu": MenuView|null, "chrome": ChromeView, "look": HostLook,
            "pointer": {"x":..,"y":..}|null,           // highlight_hovered-row
            "selection": {"anchor": {"x":..,"y":..}, "head": {"x":..,"y":..},
                          "bound": {"x":..,"y":..,"width":..,"height":..}}|null },  // highlight_selection
  "rows": [ [ {cell}, ... ], ... ] }                  // H rows of W cells
```

A cell is `{"ch": "x", "fg": "slot", "bg": "slot", "mods": ["bold"]}` with every default left out: no `ch` is a
space, no `fg` is the canvas foreground, no `bg` the canvas background, no `mods` none. Colours are the theme's slot
names (`accent.primary`, `bg.selection`); where slots share one colour the names are joined by `/`. A colour with no
slot is `#rrggbb`. `colours` says each name's `#rrggbb`. The cell right of a two-column character has `"ch": ""`.
The theme is the default look at the true-colour tier.

`highlight_*` scenes are one state each: the selected row, the focused pane, the row the pointer is over, a menu with
its selection, text selected by a drag, a filter being typed. Each carries the view it was drawn from (`view`).
