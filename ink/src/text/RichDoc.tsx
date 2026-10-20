import React from "react";
import { Box, Text as InkText } from "ink";
import type { Inline, Marker, RichBlock, Seg, Tone } from "../nodes/view.js";
import { SegRun, textProps } from "./TextLine.js";
import { specStyle } from "./look.js";

/** ui::rich::Look: the slot each part of a document is drawn in. */
const look = (spec: string, plain?: boolean) =>
  plain ? {} : textProps(specStyle(spec));
import { emojiOf } from "./emoji.js";
import { MediaBlock } from "../media/index.js";
import { cellWidth } from "../chrome/width.js";
import { toneInk } from "./theme.js";
import { textPieces, trimPieces, wrapPieces, type Piece } from "./wrap.js";

/** A document's inline run as the view segment it draws as. Marks the Ink segment cannot carry (italic, strike)
 * are drawn by the run component below; this maps the ink, link and chip parts. */
function inlineSeg(run: Inline, shown?: string): Seg {
  const kind = run.kind ?? "text";
  const seg: Seg = { text: shown ?? run.text, strong: run.marks?.bold };
  // A piece of a run already turned into what it shows (wrapped) keeps its text; its look is chosen all the same.
  if (shown === undefined && kind === "emoji") seg.text = emojiOf(run.text) ?? run.text;
  else if (typeof kind === "object") {
    if ("link" in kind) {
      seg.link = kind.link.url;
      seg.look = "accent.primary underline";
    } else if ("mention" in kind) {
      seg.look = kind.mention.me ? "status.warning bold" : "accent.tertiary";
    } else if ("badge" in kind) {
      seg.ink = toneInk(kind.badge.tone);
      seg.chip = true;
    } else if ("image" in kind) {
      seg.look = "text.muted";
      if (shown === undefined) seg.text = `[image: ${run.text || kind.image.src}]`;
    }
  }
  if (run.marks?.code) seg.look = "accent.secondary";
  else if (kind === "emoji") seg.look = "text.secondary";
  else if (!seg.look && !seg.chip)
    seg.look = seg.strong ? "text.emphasis" : "text.primary";
  return seg;
}

/** What a run shows as text: emoji by name, an image as its tag. */
const shownText = (run: Inline): string => inlineSeg(run).text ?? "";

/** ui::rich::wrap over a paragraph's runs, one Ink line per wrapped line (a Text per line, cut at the edge), so the
 * document breaks where the host's wrap does. Without a width the Box's own wrap draws it. */
function WrappedInlines({ runs, plain, width, prefix, style }: { runs: Inline[]; plain?: boolean; width?: number; prefix?: string; style?: object }) {
  if (!width) {
    return (
      <InkText wrap="wrap" {...style}>
        {prefix}
        <Inlines runs={runs} plain={plain} />
      </InkText>
    );
  }
  const pieces: Piece<number>[] = [];
  runs.forEach((run, i) => {
    if (run.kind === "break") pieces.push({ kind: "break" });
    else pieces.push(...textPieces(shownText(run), i));
  });
  const lines = wrapPieces(trimPieces(pieces), width - (prefix?.length ?? 0));
  return (
    <>
      {lines.map((line, n) => (
        <InkText key={n} wrap="truncate" {...style}>
          {n === 0 ? prefix : prefix ? " ".repeat(prefix.length) : null}
          <Inlines runs={line.map((r) => ({ ...runs[r.tag]!, text: r.text }))} plain={plain} shown />
        </InkText>
      ))}
    </>
  );
}

export function Inlines({ runs, plain, shown }: { runs: Inline[]; plain?: boolean; shown?: boolean }) {
  return (
    <>
      {runs.map((run, i) => {
        if (run.kind === "break") return <InkText key={i}>{"\n"}</InkText>;
        const seg = inlineSeg(run, shown ? run.text : undefined);
        const m = run.marks;
        if (!plain && (m?.italic || m?.strike || m?.underline)) {
          return (
            <InkText
              key={i}
              italic={m.italic}
              strikethrough={m.strike}
              underline={m.underline}
            >
              <SegRun seg={seg} />
            </InkText>
          );
        }
        return <SegRun key={i} seg={seg} plain={plain} />;
      })}
    </>
  );
}

export function markerText(marker: Marker, depth: number): string {
  if (typeof marker === "object") return `${marker.number}.`;
  switch (marker) {
    case "bullet":
      return depth > 0 ? "◦" : "•";
    case "todo":
      return "☐";
    case "done":
      return "☑";
    case "decision":
      return "◇";
  }
}

const TONE_SYMBOL: Record<Tone, string> = {
  plain: "│",
  muted: "│",
  accent: "ℹ",
  info: "ℹ",
  good: "✓",
  warn: "⚠",
  bad: "✗",
};

function words(runs: Inline[]): string {
  return runs.map((r) => (r.kind === "break" ? " " : r.text)).join("");
}

/** One block of a document, wrapped by Ink to the Box that holds it. */
/** ui::rich::Layout::sep: a blank row before every block but the first (a list item's first block has none), so
 * blocks that follow each other are a row apart. */
export function RichBlocks({
  blocks,
  plain,
  depth = 0,
  width,
}: {
  blocks: RichBlock[];
  plain?: boolean;
  depth?: number;
  /** The cells the blocks have; with it paragraphs break as ui::rich does, without it Ink's wrap does. */
  width?: number;
}) {
  return (
    <>
      {blocks.map((b, j) => (
        <React.Fragment key={j}>
          {j > 0 ? <InkText> </InkText> : null}
          <RichBlockView block={b} plain={plain} depth={depth} width={width} />
        </React.Fragment>
      ))}
    </>
  );
}

export function RichBlockView({
  block,
  plain,
  depth = 0,
  width,
}: {
  block: RichBlock;
  plain?: boolean;
  depth?: number;
  width?: number;
}) {
  switch (block.t) {
    case "para":
      return (
        <WrappedInlines runs={block.text} plain={plain} width={width} />
      );
    case "heading":
      return (
        <Box flexDirection="column">
          <WrappedInlines
            runs={block.text}
            plain={plain}
            width={width}
            prefix={plain ? `${"#".repeat(block.level)} ` : undefined}
            style={plain ? { bold: true } : look("text.emphasis bold")}
          />
          {block.level === 1 && !plain ? (
            <InkText {...look("text.muted")}>
              {"─".repeat(Math.max(3, Math.min(40, words(block.text).length)))}
            </InkText>
          ) : null}
        </Box>
      );
    case "list":
      return (
        <Box flexDirection="column">
          {block.items.map((item, i) => (
            <Box key={i}>
              <Box
                width={markerText(item.marker, depth).length + 1}
                flexShrink={0}
              >
                <InkText {...look("text.secondary", plain)}>
                  {markerText(item.marker, depth)}{" "}
                </InkText>
              </Box>
              <Box flexDirection="column" flexGrow={1}>
                <RichBlocks blocks={item.blocks} plain={plain} depth={depth + 1} width={width === undefined ? undefined : Math.max(1, width - markerText(item.marker, depth).length - 1)} />
              </Box>
            </Box>
          ))}
        </Box>
      );
    case "quote":
      return (
        <Box>
          <InkText {...look("accent.muted", plain)}>│ </InkText>
          <Box flexDirection="column" flexGrow={1}>
            <RichBlocks blocks={block.blocks} plain={plain} depth={depth} width={width === undefined ? undefined : Math.max(1, width - 2)} />
          </Box>
        </Box>
      );
    case "callout": {
      const symbol = block.symbol || TONE_SYMBOL[block.tone];
      return (
        <Box>
          <InkText {...look(tonedSlot(block.tone), plain)}>{symbol} </InkText>
          <Box flexDirection="column" flexGrow={1}>
            <RichBlocks blocks={block.blocks} plain={plain} depth={depth} width={width === undefined ? undefined : Math.max(1, width - cellWidth(symbol) - 1)} />
          </Box>
        </Box>
      );
    }
    case "code":
      return (
        <Box flexDirection="column">
          {block.lang ? (
            <InkText {...look("text.muted", plain)}>{block.lang}</InkText>
          ) : null}
          {block.text.split("\n").map((l, i) => (
            <InkText
              key={i}
              {...look("accent.secondary", plain)}
            >{`  ${l}`}</InkText>
          ))}
        </Box>
      );
    case "table":
      return <RichTable block={block} plain={plain} />;
    case "rule":
      return <InkText {...look("text.muted", plain)}>{"─".repeat(20)}</InkText>;
    case "details":
      return (
        <Box flexDirection="column">
          <InkText {...(plain ? { bold: true } : look("text.secondary bold"))}>
            <Inlines runs={block.summary} plain={plain} />
          </InkText>
          <Box marginLeft={2} flexDirection="column">
            <RichBlocks blocks={block.blocks} plain={plain} depth={depth} width={width === undefined ? undefined : Math.max(1, width - 2)} />
          </Box>
        </Box>
      );
    case "media":
      // A document's own picture or file, as ui::rich draws it where no picture is placed (media/MediaBlock).
      return <MediaBlock media={block} plain={plain} />;
  }
}

/** ui::rich::Look::tones: the slot of each tone. */
function tonedSlot(tone: Tone): string {
  return {
    plain: "text.primary",
    muted: "text.muted",
    accent: "accent.primary",
    info: "status.info",
    good: "status.success",
    warn: "status.warning",
    bad: "status.error",
  }[tone];
}

/** A table: columns padded to their widest cell, cut at the pane's edge by Ink when they do not fit. */
function RichTable({
  block,
  plain,
}: {
  block: Extract<RichBlock, { t: "table" }>;
  plain?: boolean;
}) {
  const head = block.head ?? [];
  const widths = head.map((h, c) =>
    Math.max(
      words(h).length,
      ...block.rows.map((r) => words(r[c] ?? []).length),
    ),
  );
  const cell = (runs: Inline[], c: number) => {
    const w = words(runs);
    const pad = Math.max(0, (widths[c] ?? w.length) - w.length);
    const align = block.aligns?.[c] ?? "left";
    return align === "right"
      ? " ".repeat(pad) + w
      : align === "center"
        ? " ".repeat(pad >> 1) + w + " ".repeat(pad - (pad >> 1))
        : w + " ".repeat(pad);
  };
  const cols = head.length || block.rows[0]?.length || 0;
  return (
    <Box flexDirection="column">
      {head.length ? (
        <InkText {...(plain ? {} : look("text.muted bold"))}>
          {Array.from({ length: cols }, (_, c) => cell(head[c] ?? [], c)).join(
            "  ",
          )}
        </InkText>
      ) : null}
      {block.rows.map((r, i) => (
        <InkText key={i}>
          {Array.from({ length: cols }, (_, c) => cell(r[c] ?? [], c)).join(
            "  ",
          )}
        </InkText>
      ))}
    </Box>
  );
}
