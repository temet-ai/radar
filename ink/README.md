# radar-ink

radar's Ink renderer. The Rust host (`radar`) starts it as a child process when the Ink renderer is
selected (`RADAR_RENDERER=ink`). While it runs it owns the TTY (stdin and stdout are inherited).
It learns everything over the Unix domain socket named in `RADAR_IPC_SOCKET` (one JSON object per
newline-terminated UTF-8 line; protocol in `crates/radar-core/src/ipc.rs`) and never reads radar's config,
state or journal files, opens no database and calls no provider. It exits non-zero on any error so
the host's supervisor can restart it.

Exit codes: `0` normal (host sent Shutdown or the user quit), `2` `RADAR_IPC_SOCKET` is not set,
`3` the host refused the protocol version, `1` any other error (including a lost connection).

## The bundle (what the host and the installer follow)

`npm run bundle` writes **one file**, `dist/radar-ink.js`, an ES module that runs with
`node dist/radar-ink.js` and nothing beside it: no `node_modules`, and Yoga's wasm is inlined
(ink 8's `yoga-layout` ships it as base64 JavaScript, which esbuild bundles). The exact file set
to ship is therefore `radar-ink.js` alone. `bundle.test.ts` runs a copy of it alone in an empty folder.

The host looks for the bundle, in this order:

1. `RADAR_INK_BUNDLE` (a path to the file),
2. `<directory of the radar executable>/../share/radar/ink/radar-ink.js` (where `scripts/install` copies it),
3. the checkout's `ink/dist/radar-ink.js` (walk up from the executable to a folder holding `Cargo.toml` and `ink/`),

and runs it as `node <file>`; `node` must be on `PATH` (Node 22 or newer). A missing node or
bundle makes the host print one line and run the Ratatui renderer.

Two details of the bundle script, both needed to make a single file work:

- `--alias:react-devtools-core=./src/transport/devtools-stub.ts`: Ink statically imports
  `react-devtools-core` (only used when `DEV=true`); the alias replaces it with an empty stub.
- `src/main.tsx` imports `./transport/require-shim.js` first: the bundle contains CommonJS
  dependencies that call `require("assert")`, which ESM does not have.

Without a TTY on stdout, Ink writes its frame when it unmounts (so tests that read piped output
stop the process first).

## Use

```
cd ink
npm ci            # npm install while package-lock.json is not committed
npm run typecheck # tsc --noEmit
npm test          # vitest run (test/**/*.test.ts and .tsx)
npm run bundle    # dist/radar-ink.js
npm start         # node dist/radar-ink.js (needs RADAR_IPC_SOCKET)
```

Ink tests render at a width with `ink-testing-library` and a stdout stub that has `columns`.
npm only; add a dependency only from `ink/` and commit `package.json` and `package-lock.json`
together. The package carries no sqlite, provider SDK or HTTP client.

## Transport

`src/transport/connection.ts`: `connect(path): Promise<Connection>` over `node:net`;
`Connection` has `send(message)`, `onMessage(handler)`, `onClose(handler)` and `close()`. Lines are
reassembled across chunks (multi-byte characters may be split), invalid JSON closes with reason
`invalid json`, and a line over 16 MiB closes with reason `line too long`.
`src/transport/fake.ts`: `FakeConnection` for tests, with `sent: unknown[]`, `receive(message)` and
`drop(reason)`.

## Folders

| Folder | Owner |
|---|---|
| `src/transport`, `test/transport`, `test/bundle` | T2 |
| `src/protocol` (generated from the Rust types) | T3 |
| `src/app`, `src/chrome`, `src/input`, `src/state` | T10 |
| `src/nodes`, `src/editor`, `src/text`, `test/fixtures` | T11 |
| `src/workarea`, `src/media` | T12 |
