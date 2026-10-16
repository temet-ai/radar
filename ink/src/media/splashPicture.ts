// The splash's scope as a picture (crates/radar-core/src/splash.rs kitty_show, kitty_at, kitty_hide, clear_under): where
// Ink's Hello said `image: "kitty"` the host sends `SplashView.picture`, a PNG to lay over the scope's cells. Each frame
// replaces the last (the same image id and placement), the cells under it are blanked, and when the splash goes the
// picture is deleted once.
import type { SplashPicture } from "../protocol/generated/SplashPicture.js";
import type { SplashRun } from "../protocol/generated/SplashRun.js";
import { charWidth } from "../chrome/width.js";

const ESC = "\x1b";

/** splash::kitty_show: transmit and display the PNG at the cursor, sized to its cells, `C=1` (the cursor stays), as image
 * `id` placement 1, the base64 in chunks of 4096. */
export function kittySplash(pic: SplashPicture): string {
  let out = "";
  const n = Math.ceil(pic.png.length / 4096);
  for (let i = 0; i < n; i++) {
    const more = i + 1 < n ? 1 : 0;
    const head = i === 0 ? `a=T,f=100,i=${pic.id},p=1,C=1,c=${pic.cols},r=${pic.rows},q=2,` : "q=2,";
    out += `${ESC}_G${head}m=${more};${pic.png.slice(i * 4096, (i + 1) * 4096)}${ESC}\\`;
  }
  return out;
}

/** splash::kitty_at: [`kittySplash`] at the picture's cell (1-based on the wire), the cursor saved and put back. */
export function kittySplashAt(pic: SplashPicture): string {
  return `${ESC}7${ESC}[${pic.y + 1};${pic.x + 1}H${kittySplash(pic)}${ESC}8`;
}

/** splash::kitty_hide: take the picture of image `id` away. */
export function kittySplashHide(id: number): string {
  return `${ESC}_Ga=d,d=I,i=${id},q=2${ESC}\\`;
}

/** splash::clear_under: the cells the picture lies over (cols by rows at x, y) are spaces, in the colours they had, so no
 * braille shows round its dots. */
export function blankUnder(rows: SplashRun[][], pic: SplashPicture): SplashRun[][] {
  return rows.map((runs, y) => {
    if (y < pic.y || y >= pic.y + pic.rows) return runs;
    const out: SplashRun[] = [];
    let x = 0;
    for (const r of runs) {
      let text = "";
      for (const ch of r.text) {
        const w = charWidth(ch.codePointAt(0)!);
        text += x >= pic.x && x < pic.x + pic.cols ? " ".repeat(w) : ch;
        x += w;
      }
      out.push({ ...r, text });
    }
    return out;
  });
}
