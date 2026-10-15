import React, { useEffect } from "react";
import { Box, Text } from "ink";
import type { Capabilities } from "../protocol/generated/Capabilities.js";
import { kittyImage } from "./kitty.js";

/**
 * The media artifact a part declares: `Extension { capability: "media", version: 1, data }`
 * (`data` is this shape; the extension's `fallback` is a plain document for hosts that
 * do not know "media"). Bounded: the preview, when present, is a PNG of at most 256 KiB.
 */
export interface MediaData {
  /** Artifact reference, e.g. "sha256:ab12…". */
  digest: string;
  mime: string;
  size: number;
  alt: string;
  source: string;
  /** Action id that opens the artifact externally (Intent::Invoke). */
  open_action: string;
  open_label?: string;
  /** The key that runs the action, as bars show it elsewhere ("o"); the action id itself is never drawn. */
  open_key?: string;
  /** Video: the time the external player should start at, in seconds. */
  at?: number;
  /** PNG preview, base64. Shown only when the terminal negotiated an image protocol. */
  preview_png?: string;
  /** Cells the preview takes. */
  preview_cells?: { cols: number; rows: number };
}

export function humanSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

export function isVideo(mime: string): boolean {
  return mime.startsWith("video/");
}

export interface MediaSurfaceProps {
  media: MediaData;
  capabilities: Pick<Capabilities, "image">;
  /** Where the surface sits on the terminal (1-based), for a preview. Without it no image is drawn. */
  origin?: { row: number; col: number };
  /** The raw stdout writer: escape sequences go here, never through Ink's text. */
  write?: (data: string) => void;
}

/** Whether a preview will be written for these inputs: kitty negotiated, a preview given, a place known. */
export function willDrawImage(p: MediaSurfaceProps): boolean {
  return p.capabilities.image === "kitty" && !!p.media.preview_png && !!p.origin && !!p.write;
}

/**
 * Always: what it is, the alt text, the source and the external-open action.
 * A Kitty image is added only when `capabilities.image === "kitty"`. Video is
 * metadata and the external-player action (no in-terminal playback is claimed).
 */
export function MediaSurface(props: MediaSurfaceProps) {
  const { media } = props;
  const draw = willDrawImage(props);
  const cells = media.preview_cells ?? { cols: 20, rows: 8 };
  useEffect(() => {
    if (draw) props.write!(kittyImage(media.preview_png!, props.origin!, cells.cols, cells.rows));
  }, [draw, media.digest, media.preview_png, props.origin?.row, props.origin?.col]);
  const video = isVideo(media.mime);
  const at = video && media.at !== undefined ? ` at ${media.at}s` : "";
  return (
    <Box flexDirection="column">
      <Text bold>
        {video ? "video" : "media"} · {media.mime} · {humanSize(media.size)} · {media.digest}
      </Text>
      <Text>alt: {media.alt || "(no description)"}</Text>
      <Text>source: {media.source}</Text>
      {draw ? (
        // Reserve the preview's cells so layout below does not move.
        <Box height={cells.rows} width={cells.cols} />
      ) : null}
      <Text>
        {media.open_key ?? "enter"} {media.open_label ?? (video ? "open in external player" : "open externally")}
        {at}
      </Text>
    </Box>
  );
}
