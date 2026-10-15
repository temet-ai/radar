import React from "react";
import { Text as InkText } from "ink";
import type { Media } from "../protocol/generated/Media.js";
import { specStyle } from "../text/look.js";
import { textProps } from "../text/TextLine.js";

const look = (spec: string, plain?: boolean) => (plain ? {} : textProps(specStyle(spec)));

/**
 * A document's own image or file, as ui::rich::Layout::media draws it where no picture is placed (a basic
 * terminal, or no picture fetched): `▣ image <name> <url>` or `▤ file <name> <url>`. The label and the name
 * are in the marker slot (text.secondary), the url in the link slot (accent.primary underline).
 * A picture itself is Kitty-only and needs the host to send the bytes; the protocol has no such message yet.
 */
export function mediaLine(m: Media): string {
  const label = m.image ? "▣ image" : "▤ file";
  return [label, m.name?.trim() ? m.name : "", m.url ?? ""].filter((s) => s !== "").join(" ");
}

export function MediaBlock({ media, plain }: { media: Media; plain?: boolean }) {
  const label = m_label(media);
  const name = media.name?.trim() ? ` ${media.name}` : "";
  return (
    <InkText wrap="wrap">
      <InkText {...look("text.secondary", plain)}>
        {label}
        {name}
      </InkText>
      {media.url ? (
        <>
          <InkText> </InkText>
          <InkText {...look("accent.primary underline", plain)}>{media.url}</InkText>
        </>
      ) : null}
    </InkText>
  );
}

const m_label = (m: Media) => (m.image ? "▣ image" : "▤ file");
