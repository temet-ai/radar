import React from "react";
import { Box, Text } from "ink";
import type { SplashView } from "../protocol/generated/SplashView.js";

/**
 * The startup splash (crates/radar-core/src/splash.rs): the host works out every frame as rows of runs across the whole
 * screen, with their colours resolved, and this draws them as they come, so Ink draws the cells Ratatui paints. Keys
 * and clicks still go to the screen under it (the snapshot carries it); the host ends the splash on the first one.
 */
export function Splash({ view, columns, rows }: { view: SplashView; columns: number; rows: number }) {
  return (
    <Box flexDirection="column" width={columns} height={rows} overflow="hidden">
      {view.rows.slice(0, rows).map((runs, y) => (
        <Text key={y} wrap="truncate">
          {runs.map((r, i) => (
            <Text key={i} color={r.fg || undefined} backgroundColor={r.bg || undefined} bold={r.bold || undefined}>
              {r.text}
            </Text>
          ))}
        </Text>
      ))}
    </Box>
  );
}
