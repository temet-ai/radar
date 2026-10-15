import React from "react";
import { describe, expect, it } from "vitest";
import { render } from "ink-testing-library";
import { MediaSurface, type MediaData } from "../../src/media/MediaSurface.js";
import { kittyImage } from "../../src/media/kitty.js";

const png = Buffer.from("not-really-a-png-but-bounded".repeat(300)).toString("base64"); // > 4096 b64 chars
const image: MediaData = {
  digest: "sha256:ab12cd",
  mime: "image/png",
  size: 20480,
  alt: "Pipeline graph with one failed job",
  source: "https://gitlab.example/g/p/-/artifacts/7",
  open_action: "work.media.open",
  open_key: "o",
  preview_png: png,
  preview_cells: { cols: 20, rows: 4 },
};
const video: MediaData = { ...image, mime: "video/mp4", size: 5 * 1024 * 1024, at: 42, preview_png: undefined, open_action: "work.media.play" };

function run(media: MediaData, imageProto: "kitty" | "sixel" | "none", origin?: { row: number; col: number }) {
  const written: string[] = [];
  const r = render(<MediaSurface media={media} capabilities={{ image: imageProto }} origin={origin} write={(d) => written.push(d)} />);
  const f = r.lastFrame() ?? "";
  r.unmount();
  return { f, written };
}

describe("media surface", () => {
  for (const proto of ["none", "sixel"] as const) {
    it(`${proto}: metadata, alt, source and open action; no escape sequence written`, () => {
      const { f, written } = run(image, proto, { row: 3, col: 5 });
      expect(f).toMatchSnapshot();
      for (const t of ["image/png", "20.0 KiB", "sha256:ab12cd", "alt: Pipeline graph", "source: https://gitlab.example", "o open externally"])
        expect(f).toContain(t);
      expect(written).toEqual([]);
      expect(f).not.toContain("\x1b_G");
    });
  }

  it("kitty: same text plus one image written outside the text frame", () => {
    const { f, written } = run(image, "kitty", { row: 3, col: 5 });
    expect(f).not.toContain("\x1b");
    expect(f).toContain("alt: Pipeline graph");
    expect(written).toHaveLength(1);
    expect(written[0]).toMatch(/^\x1b7\x1b\[3;5H\x1b_Ga=T,f=100,C=1,c=20,r=4,q=2,m=1;/);
    expect(written[0]!.endsWith("\x1b\\\x1b8")).toBe(true);
  });

  it("kitty without a place or a preview still shows text only", () => {
    expect(run(image, "kitty").written).toEqual([]);
    expect(run({ ...image, preview_png: undefined }, "kitty", { row: 1, col: 1 }).written).toEqual([]);
  });

  it("video: metadata and the external-player action, even under kitty", () => {
    const { f, written } = run(video, "kitty", { row: 1, col: 1 });
    expect(f).toMatchSnapshot();
    expect(f).toContain("o open in external player at 42s");
    expect(written).toEqual([]);
  });

  it("kitty chunks the payload at 4096 and ends with m=0", () => {
    const s = kittyImage(png, { row: 1, col: 1 }, 2, 2);
    const chunks = s.match(/\x1b_G[^\x1b]*\x1b\\/g)!;
    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks[0]).toContain("m=1;");
    expect(chunks.at(-1)).toContain("m=0;");
    expect(chunks.every((c) => c.length < 4096 + 64)).toBe(true);
  });
});
