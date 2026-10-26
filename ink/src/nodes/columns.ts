import type { ColumnSpec } from "./view.js";

/** A column placed in a row. `width` counts the cell of air after the value, as ui::Column::width does, so the
 * value has `width - 1` cells and the columns sit edge to edge. */
export interface Placed {
  column: ColumnSpec;
  width: number;
}

const total = (cs: { width: number }[]) => cs.reduce((n, c) => n + c.width, 0);

/** Columns the user dragged wider than their part made them (`natural`, layout.rs) give the cells back, the widest
 * first, until `over` cells are found or none is wider than its part's width: room is never made by dropping a
 * column, or by drawing past the edge, while a dragged one can give it. */
export function giveBack(placed: Placed[], over: number): Placed[] {
  if (over <= 0) return placed;
  const extra = (p: Placed) => Math.max(0, p.width - Math.max(MIN_WIDTH, p.column.natural ?? p.width));
  const order = placed.map((p, i) => i).sort((a, b) => extra(placed[b]!) - extra(placed[a]!));
  const out = [...placed];
  for (const i of order) {
    if (over <= 0) break;
    const give = Math.min(over, extra(out[i]!));
    if (give > 0) {
      out[i] = { ...out[i]!, width: out[i]!.width - give };
      over -= give;
    }
  }
  return out;
}
const MIN_WIDTH = 3;

/** ui::Columns::fit: while the columns and a name of at least `name` cells do not fit `width`, a dragged column gives
 * its extra back, then the lowest `keep` goes (the later of equals first); the order stays; the last one left is kept
 * whatever the width. */
function fit(columns: ColumnSpec[], width: number, name: number): Placed[] {
  let cols: Placed[] = columns.map((column) => ({ column, width: column.width }));
  for (;;) {
    cols = giveBack(cols, total(cols) + name - width);
    if (cols.length <= 1 || total(cols) + name <= width) return cols;
    let drop = 0;
    cols.forEach((c, i) => {
      if (c.column.keep <= cols[drop]!.column.keep) drop = i;
    });
    // what the dropped column took is given back to the rest at their dragged widths, then taken again if need be
    cols = columns.filter((c) => c !== cols[drop]!.column && cols.some((p) => p.column === c)).map((column) => ({ column, width: column.width }));
  }
}

/** The columns of a tree in `width` cells beside names of at least `name` (ui view_tui::draw_tree): with
 * `ordered`, every column in declared order fitted by keep; else the part's own columns are fitted and the extra
 * (contributed) ones are put before the last while the names keep their room. */
export function fitTreeColumns(
  columns: ColumnSpec[],
  ordered: boolean,
  width: number,
  name: number,
): Placed[] {
  if (ordered) return fit(columns, width, name);
  const own = fit(
    columns.filter((c) => !c.extra),
    width,
    name,
  );
  let used = total(own) + name;
  const add: Placed[] = [];
  for (const c of columns.filter((c) => c.extra)) {
    // a contributed column dragged wider comes at what is left, down to its part's width
    const w = Math.min(c.width, width - used);
    if (w >= Math.min(c.width, c.natural ?? c.width) && w > 0) {
      add.push({ column: c, width: w });
      used += w;
    }
  }
  const at = Math.max(0, own.length - 1);
  return [...own.slice(0, at), ...add, ...own.slice(at)];
}

/** The columns of a table in `width` cells (ui::table::Table): none is dropped. Every column goes to the layout
 * (`Min` for the flex column, `Length` for the rest, one cell of spacing) and `solveWidths` is that layout. A placed
 * width counts the cell of air after the value, as ui::Column::width does. */
export function fitTableColumns(
  columns: ColumnSpec[],
  width: number,
): Placed[] {
  const flex = columns.findIndex((c) => c.flex);
  // a column dragged wider gives its extra back first; the layout shares out any shortfall after that
  const want = columns.reduce((n, c) => n + Math.max(0, c.width - 1), 0);
  const given = giveBack(columns.map((column) => ({ column, width: column.width })), want - Math.max(0, width - (columns.length - 1)));
  const values = solveWidths(
    given.map((p) => Math.max(0, p.width - 1)),
    flex < 0 ? undefined : flex,
    width,
  );
  return columns.map((column, i) => ({ column, width: values[i] + 1 }));
}

/** Minimise `cost . x` over `x >= 0` with `rows . x = rhs` (every `rhs >= 0`): two-phase simplex, Bland's rule. */
function simplex(cost: number[], rows: number[][], rhs: number[]): number[] {
  const m = rows.length;
  const n = cost.length;
  const eps = 1e-9;
  // tableau: columns 0..n-1 the variables, n..n+m-1 the artificials, last the right side
  const t = rows.map((r, i) => [
    ...r,
    ...rows.map((_, j) => (i === j ? 1 : 0)),
    rhs[i],
  ]);
  const basis = rows.map((_, i) => n + i);
  const run = (obj: number[], width: number) => {
    // obj is the cost row over the first `width` columns
    for (;;) {
      const z = obj.map((c, j) => {
        let r = c;
        basis.forEach((b, i) => (r -= obj[b] * t[i][j]));
        return r;
      });
      const enter = z.findIndex((v, j) => j < width && v < -eps);
      if (enter < 0) return;
      let leave = -1;
      let ratio = Infinity;
      for (let i = 0; i < m; i++) {
        if (t[i][enter] > eps) {
          const r = t[i][n + m] / t[i][enter];
          if (r < ratio - eps || (Math.abs(r - ratio) <= eps && basis[i] < basis[leave])) {
            ratio = r;
            leave = i;
          }
        }
      }
      if (leave < 0) return;
      const pv = t[leave][enter];
      t[leave] = t[leave].map((v) => v / pv);
      for (let i = 0; i < m; i++) {
        if (i !== leave && Math.abs(t[i][enter]) > eps) {
          const f = t[i][enter];
          t[i] = t[i].map((v, j) => v - f * t[leave][j]);
        }
      }
      basis[leave] = enter;
    }
  };
  const phase1 = [...cost.map(() => 0), ...rows.map(() => 1), 0];
  run(phase1, n + m);
  // an artificial left in the basis at zero is pivoted out when it can be
  basis.forEach((b, i) => {
    if (b >= n) {
      const j = t[i].findIndex((v, k) => k < n && Math.abs(v) > eps);
      if (j >= 0) {
        const pv = t[i][j];
        t[i] = t[i].map((v) => v / pv);
        for (let r = 0; r < m; r++)
          if (r !== i && Math.abs(t[r][j]) > eps) {
            const f = t[r][j];
            t[r] = t[r].map((v, k) => v - f * t[i][k]);
          }
        basis[i] = j;
      }
    }
  });
  run([...cost, ...rows.map(() => 1e9), 0], n);
  const x = cost.map(() => 0);
  basis.forEach((b, i) => {
    if (b < n) x[b] = t[i][n + m];
  });
  return x;
}

/** The widths `Layout::horizontal(widths).flex(Start).spacing(1)` gives a table's columns in `width` cells.
 * `values[i]` is a column's value width (`ColumnSpec.width - 1`: the air cell is the spacing) and `flex` the index of
 * the column given `Constraint::Min`; every other is `Constraint::Length`. The solver (cassowary) weighs,
 * strongest first: the one-cell spacers, `Min` at least its value, each `Length` its value, the flex column growing
 * into what is left, and last (ALL_SEGMENT_GROW) that neighbouring columns match in width. So when all fit the flex
 * column takes what is left; when they do not it keeps its minimum and the `Length`s shrink by the least total, spread
 * to make neighbouring columns differ least. That spread is real-valued; the columns' edges are then rounded as
 * `changes_to_rects` does. */
export function solveWidths(
  values: number[],
  flex: number | undefined,
  width: number,
): number[] {
  const n = values.length;
  if (n === 0) return [];
  const room = Math.max(0, width - (n - 1));
  const fixed = values.reduce((s, v, i) => (i === flex ? s : s + v), 0);
  const min = flex === undefined ? 0 : values[flex];
  if (fixed + min <= room)
    return values.map((v, i) => (i === flex ? room - fixed : v));
  const pinned = flex === undefined ? 0 : Math.min(min, room);
  // w = u (lengths, 0..value, summing to what the flex column leaves) with the flex column pinned. Variables:
  // u_0..u_{n-1}, slack_0..slack_{n-1} (u + slack = cap), p_i, q_i for i < n-1 (w_i - w_{i+1} = p_i - q_i).
  const cap = values.map((v, i) => (i === flex ? 0 : v));
  const pin = (i: number) => (i === flex ? pinned : 0);
  const nv = 2 * n + 2 * (n - 1);
  const row = () => new Array<number>(nv).fill(0);
  const rows: number[][] = [];
  const rhs: number[] = [];
  for (let i = 0; i < n; i++) {
    const r = row();
    r[i] = 1;
    r[n + i] = 1;
    rows.push(r);
    rhs.push(cap[i]);
  }
  const sum = row();
  for (let i = 0; i < n; i++) sum[i] = 1;
  rows.push(sum);
  rhs.push(room - pinned);
  for (let i = 0; i < n - 1; i++) {
    const r = row();
    r[i] = 1;
    r[i + 1] = -1;
    r[2 * n + 2 * i] = -1;
    r[2 * n + 2 * i + 1] = 1;
    const b = pin(i + 1) - pin(i);
    rows.push(b < 0 ? r.map((v) => -v) : r);
    rhs.push(Math.abs(b));
  }
  // ties between equal spreads go to the later columns, as the solver's pivoting leaves them
  const cost = row().map((_, j) => (j >= 2 * n ? 1 : j < n ? -1e-3 * j : 0));
  const u = simplex(cost, rows, rhs);
  const real = values.map((_, i) => u[i] + pin(i));
  // edges as the layout rounds them: x * 100 to an integer, back, to an integer
  const snap = (x: number) => Math.round(Math.round(x * 100) / 100);
  let at = 0;
  return real.map((w) => {
    const start = snap(at);
    const end = snap(at + w);
    at += w + 1;
    return end - start || 0;
  });
}
