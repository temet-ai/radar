// Run under a pseudo-terminal by stdin-tty.test.ts: process.stdin is a real tty.ReadStream here, read the way
// Ink reads it; every key the tap hands the bus is written to stderr as a JSON line.
import { MouseBus, tapMouse } from "../../../src/input/stdin-tap.js";

const bus = new MouseBus();
bus.onInput((events) => {
  for (const e of events) process.stderr.write(JSON.stringify(e) + "\n");
});
process.stdin.setRawMode(true);
process.stdin.setEncoding("utf8");
tapMouse(process.stdin as any, bus);
process.stdin.on("readable", () => {
  let chunk;
  while ((chunk = process.stdin.read()) !== null) void chunk;
});
process.stderr.write("ready\n");
setTimeout(() => process.exit(0), 3000);
