// Loads the real renderer bundle with a tap on its socket and its stdout, for pty_latency.py.
// RADAR_PERF_BUNDLE: the real radar-ink.js. RADAR_PERF_LOG: a file that gets one line per event:
//   <epoch ms> <in|out|tty> <bytes> <message type or intent>
// `in` is a message from the host, `out` a message to it, `tty` a write to the terminal, `key` bytes read from stdin.
import net from "node:net";
import fs from "node:fs";
import { pathToFileURL } from "node:url";

const log = fs.openSync(process.env.RADAR_PERF_LOG, "a");
const line = (dir, bytes, what) => fs.writeSync(log, `${(performance.timeOrigin + performance.now()).toFixed(3)} ${dir} ${bytes} ${what}\n`);

const create = net.createConnection;
net.createConnection = (...args) => {
  const s = create(...args);
  const write = s.write.bind(s);
  s.write = (data, ...rest) => {
    const text = typeof data === "string" ? data : Buffer.from(data).toString("utf8");
    for (const l of text.split("\n")) {
      if (!l) continue;
      let what = "?";
      try {
        const m = JSON.parse(l);
        what = m.type === "intent" ? `intent:${m.intent?.intent}${m.intent?.chord ? ":" + JSON.stringify(m.intent.chord) : ""}` : m.type;
      } catch {}
      line("out", Buffer.byteLength(l) + 1, what);
    }
    return write(data, ...rest);
  };
  let carry = "";
  s.on("data", (chunk) => {
    const parts = (carry + chunk.toString("utf8")).split("\n");
    carry = parts.pop();
    for (const l of parts) {
      if (!l) continue;
      let what = "?";
      try {
        const m = JSON.parse(l);
        what = m.type;
      } catch {}
      line("in", Buffer.byteLength(l) + 1, what);
    }
  });
  return s;
};

const read = process.stdin.read.bind(process.stdin);
process.stdin.read = (...a) => {
  const d = read(...a);
  if (d !== null) line("key", Buffer.byteLength(d), JSON.stringify(String(d)));
  return d;
};

const tty = process.stdout.write.bind(process.stdout);
process.stdout.write = (data, ...rest) => {
  line("tty", Buffer.byteLength(data), "frame");
  return tty(data, ...rest);
};

await import(pathToFileURL(process.env.RADAR_PERF_BUNDLE).href);
