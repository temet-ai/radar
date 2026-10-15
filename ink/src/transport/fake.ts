import type { Connection } from "./connection.js";

/** In-memory Connection for tests: `sent` records what the renderer sent, `receive` plays a host message. */
export class FakeConnection implements Connection {
  sent: unknown[] = [];
  closed = false;
  private messageHandlers: Array<(m: unknown) => void> = [];
  private closeHandlers: Array<(r: string) => void> = [];

  send(message: unknown): void {
    this.sent.push(message);
  }
  onMessage(handler: (message: unknown) => void): void {
    this.messageHandlers.push(handler);
  }
  onClose(handler: (reason: string) => void): void {
    this.closeHandlers.push(handler);
  }
  close(): void {
    this.drop("closed");
  }
  receive(message: unknown): void {
    for (const h of this.messageHandlers) h(message);
  }
  /** Simulate the peer going away. */
  drop(reason: string): void {
    if (this.closed) return;
    this.closed = true;
    for (const h of this.closeHandlers) h(reason);
  }
}
