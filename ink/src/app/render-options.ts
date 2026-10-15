/** How the renderer is run: the alternate screen, the app owning Ctrl-C and console, and Ink writing only the
 * lines a frame changed (a whole-frame rewrite per key was 16 to 48 KB). test/app/frames.test.tsx holds it to a bound.
 * maxFps 120: Ink's default 30 holds a render back up to 32 ms after the last request for one, and a key's answer (the local
 * change, then the host's snapshot 10 ms later) is two renders, so the answer waited 30 to 50 ms for the second (T21). */
export const RENDER_OPTIONS = { alternateScreen: true, exitOnCtrlC: false, patchConsole: false, incrementalRendering: true, maxFps: 120 } as const;
