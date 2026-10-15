// Stands in for radar-ink.js while capture-looks.mjs runs the real host: it says hello over RADAR_IPC_SOCKET, writes the
// first `look` message the host sends to RADAR_CAPTURE_OUT, says bye and exits.
import fs from "node:fs";
import net from "node:net";

const socket = net.connect(process.env.RADAR_IPC_SOCKET);
const send = (m) => socket.write(`${JSON.stringify(m)}\n`);
let buffer = "";
socket.on("connect", () =>
  send({ type: "hello", protocol: 1, view: 1, capabilities: { columns: 80, rows: 24, mouse: true, true_colour: false, image: "none", plain_text: false } }),
);
socket.on("data", (chunk) => {
  buffer += chunk.toString("utf8");
  let nl;
  while ((nl = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, nl);
    buffer = buffer.slice(nl + 1);
    const m = JSON.parse(line);
    if (m.type === "look") {
      fs.writeFileSync(process.env.RADAR_CAPTURE_OUT, `${JSON.stringify(m, null, 2)}\n`);
      send({ type: "bye" });
      setTimeout(() => process.exit(0), 50);
    }
  }
});
socket.on("error", () => process.exit(1));
setTimeout(() => process.exit(1), 10000);
