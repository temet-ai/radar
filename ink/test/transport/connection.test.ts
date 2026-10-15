import { afterEach, describe, expect, it } from "vitest";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { connect, MAX_LINE_BYTES } from "../../src/transport/connection.js";

let cleanup: Array<() => void> = [];
afterEach(() => {
  for (const c of cleanup) c();
  cleanup = [];
});

function sockPath(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "radar-ink-"));
  cleanup.push(() => fs.rmSync(dir, { recursive: true, force: true }));
  return path.join(dir, "s.sock");
}

function serve(p: string, onConn: (s: net.Socket) => void): Promise<void> {
  return new Promise((resolve) => {
    const server = net.createServer(onConn);
    cleanup.push(() => server.close());
    server.listen(p, resolve);
  });
}

describe("connection", () => {
  it("exchanges two messages each way", async () => {
    const p = sockPath();
    const fromClient: unknown[] = [];
    await serve(p, (s) => {
      let buf = "";
      s.on("data", (d) => {
        buf += d.toString();
        let i;
        while ((i = buf.indexOf("\n")) >= 0) {
          fromClient.push(JSON.parse(buf.slice(0, i)));
          buf = buf.slice(i + 1);
        }
        if (fromClient.length === 2) {
          s.write(JSON.stringify({ n: 1 }) + "\n" + JSON.stringify({ n: 2 }) + "\n");
        }
      });
    });
    const c = await connect(p);
    const got: unknown[] = [];
    const done = new Promise<void>((r) =>
      c.onMessage((m) => {
        got.push(m);
        if (got.length === 2) r();
      }),
    );
    c.send({ a: 1 });
    c.send({ a: "é" });
    await done;
    expect(fromClient).toEqual([{ a: 1 }, { a: "é" }]);
    expect(got).toEqual([{ n: 1 }, { n: 2 }]);
    c.close();
  });

  it("reassembles a message split across chunks", async () => {
    const p = sockPath();
    const text = JSON.stringify({ word: "héllo" }) + "\n";
    const bytes = Buffer.from(text);
    await serve(p, (s) => {
      s.write(bytes.subarray(0, 9)); // splits inside the multi-byte é
      setTimeout(() => s.write(bytes.subarray(9)), 20);
    });
    const c = await connect(p);
    const m = await new Promise((r) => c.onMessage(r));
    expect(m).toEqual({ word: "héllo" });
    c.close();
  });

  it("closes with 'line too long' over 16 MiB", async () => {
    const p = sockPath();
    await serve(p, (s) => {
      s.on("error", () => {});
      const chunk = Buffer.alloc(1024 * 1024, 0x61);
      for (let i = 0; i <= MAX_LINE_BYTES / chunk.length; i++) s.write(chunk);
    });
    const c = await connect(p);
    const reason = await new Promise<string>((r) => c.onClose(r));
    expect(reason).toBe("line too long");
  });

  it("reports close when the host goes away", async () => {
    const p = sockPath();
    await serve(p, (s) => setTimeout(() => s.destroy(), 10));
    const c = await connect(p);
    const reason = await new Promise<string>((r) => c.onClose(r));
    expect(reason).toBe("closed");
  });
});
