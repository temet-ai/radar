// Usage: node style-diff.mjs <ink.ansi> <reference.ansi>
// Both files are `tmux capture-pane -e -p` output. Prints every region where the cell
// styles (fg, bg, bold, dim, italic, underline, reverse, strike) or characters differ,
// grouped per row into runs. The second file is the reference.
import { readFileSync } from 'node:fs';

const NAMED = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white'];
export function parse(text) {
  // tmux capture-pane -e carries the attributes across line ends: the state must too
  let st = {};
  return text.replace(/\n$/, '').split('\n').map((line) => {
    const cells = [];
    const re = /\x1b\[([0-9;:]*)m|([^\x1b])/gu;
    let m;
    while ((m = re.exec(line))) {
      if (m[2] !== undefined) {
        cells.push({ ch: m[2], ...st });
        continue;
      }
      const p = m[1] === '' ? [0] : m[1].split(/[;:]/).map(Number);
      for (let i = 0; i < p.length; i++) {
        const c = p[i];
        if (c === 0) st = {};
        else if (c === 1) st.bold = 1;
        else if (c === 2) st.dim = 1;
        else if (c === 3) st.italic = 1;
        else if (c === 4) st.underline = 1;
        else if (c === 7) st.reverse = 1;
        else if (c === 9) st.strike = 1;
        else if (c === 22) { delete st.bold; delete st.dim; }
        else if (c === 23) delete st.italic;
        else if (c === 24) delete st.underline;
        else if (c === 27) delete st.reverse;
        else if (c === 29) delete st.strike;
        else if (c >= 30 && c <= 37) st.fg = NAMED[c - 30];
        else if (c >= 90 && c <= 97) st.fg = 'bright' + NAMED[c - 90];
        else if (c === 39) delete st.fg;
        else if (c >= 40 && c <= 47) st.bg = NAMED[c - 40];
        else if (c >= 100 && c <= 107) st.bg = 'bright' + NAMED[c - 100];
        else if (c === 49) delete st.bg;
        else if (c === 38 || c === 48) {
          const k = c === 38 ? 'fg' : 'bg';
          if (p[i + 1] === 5) { st[k] = '#' + hex256(p[i + 2]); i += 2; }
          else if (p[i + 1] === 2) { st[k] = '#' + [p[i + 2], p[i + 3], p[i + 4]].map((n) => n.toString(16).padStart(2, '0')).join(''); i += 4; }
        }
      }
    }
    return cells;
  });
}
function hex256(n) {
  if (n < 16) {
    const t = ['000000','800000','008000','808000','000080','800080','008080','c0c0c0','808080','ff0000','00ff00','ffff00','0000ff','ff00ff','00ffff','ffffff'];
    return t[n];
  }
  if (n >= 232) { const v = 8 + (n - 232) * 10; return [v, v, v].map((x) => x.toString(16).padStart(2, '0')).join(''); }
  const i = n - 16; const lv = [0, 95, 135, 175, 215, 255];
  return [lv[Math.floor(i / 36)], lv[Math.floor(i / 6) % 6], lv[i % 6]].map((x) => x.toString(16).padStart(2, '0')).join('');
}
const key = (c) => (c ? ['fg', 'bg', 'bold', 'dim', 'italic', 'underline', 'reverse', 'strike'].map((k) => c[k] ?? '').join('|') : 'none');
const desc = (c) => (c ? ['fg', 'bg'].map((k) => (c[k] ? `${k}=${c[k]}` : '')).concat(['bold', 'dim', 'italic', 'underline', 'reverse', 'strike'].filter((k) => c[k])).filter(Boolean).join(' ') || 'default' : 'none');

export function diff(inkText, ratText) {
  const a = parse(inkText), b = parse(ratText);
  const out = [];
  for (let y = 0; y < Math.max(a.length, b.length); y++) {
    const ra = a[y] ?? [], rb = b[y] ?? [];
    const n = Math.max(ra.length, rb.length);
    let x = 0;
    while (x < n) {
      const ca = ra[x], cb = rb[x];
      // blank cells with no visible effect (space, no bg/reverse/underline) are equal whatever their fg
      const vis = (c) => (!c ? ' ' : c.ch === ' ' ? (c.bg || c.reverse || c.underline ? `sp|${c.bg ?? ''}|${c.reverse ?? ''}|${c.underline ?? ''}` : ' ') : key(c) + c.ch);
      if (vis(ca) === vis(cb)) { x++; continue; }
      let e = x + 1;
      while (e < n && vis(ra[e]) !== vis(rb[e]) && key(ra[e]) === key(ra[x]) && key(rb[e]) === key(rb[x])) e++;
      const text = (r) => r.slice(x, e).map((c) => c.ch).join('');
      out.push({ row: y + 1, col: x + 1, len: e - x, ink: desc(ca), rat: desc(cb), inkText: text(ra), ratText: text(rb) });
      x = e;
    }
  }
  return out;
}
if (process.argv[1] && process.argv[1].endsWith("style-diff.mjs") && process.argv[2] === "--rows") {
  // node style-diff.mjs --rows FROM TO FILE: the styles of rows FROM..TO (1-based) as runs of equal style
  const rows = parse(readFileSync(process.argv[5], "utf8"));
  for (let y = +process.argv[3]; y <= +process.argv[4]; y++) {
    const out = [];
    let run = null;
    for (const c of rows[y - 1] ?? []) {
      if (run && key(run.c) === key(c)) run.t += c.ch;
      else { run = { c, t: c.ch }; out.push(run); }
    }
    console.log("r" + y + ": " + out.map((r) => "[" + desc(r.c) + "] " + JSON.stringify(r.t.length > 18 ? r.t.slice(0, 16) + "…" + r.t.length : r.t)).join("  "));
  }
} else if (process.argv[1] && process.argv[1].endsWith("style-diff.mjs")) {
  const d = diff(readFileSync(process.argv[2], "utf8"), readFileSync(process.argv[3], "utf8"));
  const g = new Map();
  for (const r of d) {
    const k = r.ink + "  |  rat: " + r.rat + (r.inkText === r.ratText ? "" : " [TEXT]");
    if (!g.has(k)) g.set(k, []);
    g.get(k).push(r);
  }
  for (const [k, rs] of g) {
    const at = rs.slice(0, 4).map((r) => "r" + r.row + "c" + r.col + "+" + r.len + " " + JSON.stringify(r.inkText.slice(0, 24) + (r.inkText === r.ratText ? "" : " / " + r.ratText.slice(0, 24)))).join("; ");
    console.log("  x" + rs.length + "  ink: " + k + "\n        " + at);
  }
  console.log("  -- " + d.length + " runs, " + g.size + " groups");
}
