// The mouse pointer's shape (ui::pointer for Ink, OSC 22): a hand over what a click acts on, a text cursor over text a
// drag would select and while one does, the terminal's own arrow elsewhere. Written only when it changes and only where
// the host said so (Look.pointer); put back to the arrow when radar stops.
import { currentLook } from "../text/look.js";

export type Shape = "default" | "pointer" | "text";

/** The sequence that sets the pointer to `shape`. */
export const shapeSequence = (shape: Shape) => `\x1b]22;${shape}\x1b\\`;

export class PointerShape {
  private over: Shape = "default";
  private dragging = false;
  private written: Shape = "default";

  constructor(
    private write: (s: string) => void,
    private readonly on: () => boolean = () => currentLook().pointer ?? false,
  ) {}

  /** What the pointer is over now. */
  set(shape: Shape) {
    this.over = shape;
    this.flush();
  }

  /** A drag is selecting (the text cursor, whatever is under it), or has ended. */
  drag(dragging: boolean) {
    this.dragging = dragging;
    this.flush();
  }

  private flush() {
    const want: Shape = this.dragging ? "text" : this.over;
    if (want === this.written || !this.on()) return;
    this.written = want;
    this.write(shapeSequence(want));
  }

  /** Where the sequences go (the renderer's stdout, app/run.tsx); nowhere until then. */
  attach(write: (s: string) => void) {
    this.write = write;
  }

  /** The arrow back, if it was changed (on every way out). */
  restore() {
    if (this.written === "default") return;
    this.written = "default";
    this.write(shapeSequence("default"));
  }
}

/** The renderer's pointer: App tells it what is under the pointer, run.tsx gives it stdout and puts the arrow back. */
export const pointerShape = new PointerShape(() => {});
