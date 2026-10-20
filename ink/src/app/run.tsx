import React from "react";
import { render } from "ink";
import { connect } from "../transport/connection.js";
import { App } from "./App.js";
import { ErrorBoundary } from "./ErrorBoundary.js";
import { Session, capabilitiesFrom, EXIT_ERROR, EXIT_NO_SOCKET } from "./session.js";
import { Terminal } from "./terminal.js";
import { pointerShape } from "./pointer.js";
import { RENDER_OPTIONS } from "./render-options.js";
import { MouseBus, tapMouse } from "../input/stdin-tap.js";
import { routeHerdrRightClick } from "../input/mouse.js";
import { attachMotion } from "../motion/attach.js";
import { spawn } from "node:child_process";
import { tracer, traceTerminal } from "./trace.js";

/**
 * The renderer's entry (ink/src/main.tsx calls it). Owns the TTY while it runs; on every way out
 * (host Shutdown, Refuse, lost connection, uncaught error, signal) the terminal is restored
 * before the process exits, and any failure exits non-zero so the host's supervisor restarts it.
 */
export async function run(env: NodeJS.ProcessEnv = process.env): Promise<never> {
  const socketPath = env.RADAR_IPC_SOCKET;
  if (!socketPath) {
    process.stderr.write("radar ink renderer: RADAR_IPC_SOCKET is not set\n");
    process.exit(EXIT_NO_SOCKET);
  }
  const terminal = new Terminal(process.stdout);
  let instance: ReturnType<typeof render> | undefined;
  let exiting = false;
  const exit = (code: number, message?: string): never => {
    if (!exiting) {
      exiting = true;
      try {
        instance?.unmount();
      } catch {}
      pointerShape.restore();
      terminal.restore();
      if (message) process.stderr.write(`radar: ${message}\n`);
    }
    process.exit(code);
  };

  process.on("uncaughtException", (e) => exit(EXIT_ERROR, `renderer error: ${e.message}`));
  process.on("unhandledRejection", (e) => exit(EXIT_ERROR, `renderer error: ${String(e)}`));
  for (const sig of ["SIGTERM", "SIGHUP", "SIGINT"] as const) process.on(sig, () => exit(EXIT_ERROR));

  let session: Session;
  try {
    const conn = await connect(socketPath);
    session = new Session(conn, capabilitiesFrom(env, process.stdout), (t) => terminal.clipboard(t), true);
  } catch (e) {
    return exit(EXIT_ERROR, `cannot connect to the host: ${(e as Error).message}`);
  }
  terminal.enter();
  pointerShape.attach((s) => process.stdout.write(s));
  // In a Herdr pane the right button is Herdr's until the pane is told to pass it on (as the Ratatui runner does).
  routeHerdrRightClick(env, spawn, (err) =>
    err ? session.log("warn", err) : session.log("info", "Herdr right-clicks routed to this pane"),
  );
  // RADAR_TRACE_LATENCY=1: reads and frames are timed (before the tap, which reads through the wrapper).
  tracer.logTo((t) => session.log("info", t));
  traceTerminal(process.stdout as any, process.stdin as any);
  const mouse = new MouseBus();
  tapMouse(process.stdin, mouse);
  attachMotion(session, mouse, process.stdout);
  instance = render(
    <ErrorBoundary onError={(e) => exit(EXIT_ERROR, `renderer error: ${e.message}`)}>
      <App session={session} mouse={mouse} onExit={(code, message) => exit(code, message)} />
    </ErrorBoundary>,
    RENDER_OPTIONS,
  );
  session.start();
  return new Promise<never>(() => {});
}
