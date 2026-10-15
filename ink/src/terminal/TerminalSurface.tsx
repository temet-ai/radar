import React from "react";
import { connectSocket } from "./link.js";
import { TerminalView, type TerminalViewProps } from "./TerminalView.js";

export interface TerminalSurfaceProps extends Omit<TerminalViewProps, "open" | "openKey"> {
  /** The host's per-terminal Unix socket (the dock tab's `terminal.socket`). */
  socket: string;
}

/**
 * The dock's terminal tab: a program the host runs, drawn in `columns` x `rows` cells, with the keys and mouse of
 * the terminal going to it while `focused`. See `TerminalView` for the behaviour; this only connects it to the
 * host's socket (kept apart so stories and tests, which run without Node's `net`, use `TerminalView`).
 */
export function TerminalSurface({ socket, ...rest }: TerminalSurfaceProps) {
  return <TerminalView {...rest} open={() => connectSocket(socket)} openKey={socket} />;
}
