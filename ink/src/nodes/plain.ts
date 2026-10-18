import type { Block, Node, RichBlock, Text } from "./view.js";

const text = (segments: Text): string => segments.map((s) => (s.text ?? "") + (s.link && s.link !== s.text ? ` <${s.link}>` : "") + (s.key_hint ? ` [${s.key_hint}]` : "")).join("");
function rich(blocks: RichBlock[]): string[] {
  return blocks.flatMap((b): string[] => {
    const inline = (v: { text: string; kind?: unknown }[]) => v.map((i) => {
      if (i.kind === "break") return "\n";
      const k = i.kind as { link?: { url: string }; image?: { src: string } } | undefined;
      const url = k?.link?.url ?? k?.image?.src;
      return i.text + (url && url !== i.text ? ` <${url}>` : "");
    }).join("");
    switch (b.t) {
      case "para": case "heading": return inline(b.text).split("\n");
      case "quote": return rich(b.blocks).map((line) => `│ ${line}`);
      case "callout": return [b.symbol || b.tone, ...rich(b.blocks)];
      case "details": return [...inline(b.summary).split("\n"), ...rich(b.blocks)];
      case "list": return b.items.flatMap((i) => {
        const marker = typeof i.marker === "object" ? `${i.marker.number}.` : ({ bullet: "•", todo: "☐", done: "☑", decision: "◇" } as const)[i.marker];
        const lines = rich(i.blocks);
        return lines.map((line, n) => `${n ? " ".repeat(marker.length) : marker} ${line}`);
      });
      case "code": return b.text.split("\n");
      case "table": return [...(b.head?.length ? [b.head.map(inline).join(" | ")] : []), ...b.rows.map((r) => r.map(inline).join(" | "))].flatMap((row) => row.split("\n"));
      case "rule": return ["───"];
      case "media": return [[b.image ? "image" : "file", b.name, b.url || b.src].filter(Boolean).join(" ")];
    }
  });
}
export function plainBlocks(blocks: Block[]): string[] {
  return blocks.flatMap((b): string[] => {
    switch (b.block) {
      case "line": return (text(b.text) + (b.key ? ` [${b.key}]` : "")).split("\n");
      case "section": return [[b.title, b.detail].filter(Boolean).join(" · "), ...plainBlocks(b.blocks)];
      case "doc": return rich(b.doc).map((row) => (b.bar ? text([b.bar]) : "") + row);
      case "meter": return b.stages.map((s) => `${s.name}: ${s.mark}`);
      case "wrapped": return (text([b.prefix]) + b.text).split("\n");
    }
  });
}
/** Extensions always read their complete ordinary fallback, including offscreen action keys/reasons. */
export function plainNode(node: Node): string[] {
  if (node.node === "extension") return plainNode(node.fallback);
  if (node.node === "rich") return plainBlocks(node.blocks);
  if (node.node === "document") return node.lines.flatMap((line) => text(line).split("\n"));
  if (node.node === "state") return [node.text];
  return [];
}
