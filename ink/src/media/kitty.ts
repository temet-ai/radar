const ESC = "\x1b";

/**
 * Kitty graphics: transmit-and-display a PNG (f=100) at the cursor cell,
 * `C=1` so the cursor does not move, sized to `cols`x`rows` cells, base64
 * payload chunked at 4096 bytes. The cursor is saved and restored around it
 * so the text Ink has drawn is not disturbed.
 */
export function kittyImage(pngBase64: string, at: { row: number; col: number }, cols: number, rows: number): string {
  const chunks: string[] = [];
  for (let i = 0; i < pngBase64.length; i += 4096) chunks.push(pngBase64.slice(i, i + 4096));
  if (!chunks.length) return "";
  let out = `${ESC}7${ESC}[${at.row};${at.col}H`;
  chunks.forEach((c, i) => {
    const more = i < chunks.length - 1 ? 1 : 0;
    const head = i === 0 ? `a=T,f=100,C=1,c=${cols},r=${rows},q=2,` : "q=2,";
    out += `${ESC}_G${head}m=${more};${c}${ESC}\\`;
  });
  return out + `${ESC}8`;
}
