// radar's JavaScript runtime: the Ink renderer and the Claude Code sidecar in one
// program, built by scripts/build-runtime into a Node single executable that the
// radar binary carries. The first argument picks which one runs.
//
//   radar-js ink            the renderer (radar starts it; ink/README.md)
//   radar-js claude-agent   the Agent SDK sidecar (sidecars/claude-agent)
//
// In a single executable process.argv[1] is the executable again, as it is the
// script under `node entry.mjs`, so the role is argv[2] either way.
const role = process.argv[2];
if (role === "ink") await import("../ink/dist/radar-ink.js");
else if (role === "claude-agent") await import("../sidecars/claude-agent/src/main.js");
else {
  console.error("usage: radar-js ink|claude-agent (radar starts these itself)");
  process.exit(2);
}
