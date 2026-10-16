import { useEffect, useRef } from "react";
import type { SplashView } from "../protocol/generated/SplashView.js";
import { kittySplashAt, kittySplashHide } from "../media/splashPicture.js";

/**
 * runner.rs: after a frame with the splash's picture, write it (the same id and placement, so it replaces the last); once
 * the splash is gone after one that had a picture, write the delete once. `write` is the raw stdout writer.
 */
export function useSplashPicture(splash: SplashView | null | undefined, write: (data: string) => void) {
  const shown = useRef<number | null>(null);
  const pic = splash?.picture ?? null;
  useEffect(() => {
    if (pic) {
      write(kittySplashAt(pic));
      shown.current = pic.id;
    } else if (shown.current !== null && !splash) {
      write(kittySplashHide(shown.current));
      shown.current = null;
    }
  }, [pic?.png, pic?.id, pic?.x, pic?.y, pic?.cols, pic?.rows, !!splash]);
  // The app goes (quit, crash): the picture goes with it.
  useEffect(
    () => () => {
      if (shown.current !== null) write(kittySplashHide(shown.current));
    },
    [],
  );
}
