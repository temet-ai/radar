// node:os, node:path and node:url for the browser, as far as Ink's dependencies reach: ansi-escapes asks
// `os.release()` (for an old Windows terminal), Ink's error overview shortens stack paths with `relative` and
// `fileURLToPath`.
export const EOL = "\n";
export const release = () => "";
export const platform = () => "browser";
export const sep = "/";
export const relative = (_from: string, to: string) => to;
export const join = (...parts: string[]) => parts.join("/").replace(/\/+/g, "/");
export const fileURLToPath = (u: string | URL) => String(u).replace(/^file:\/\//, "");
export const pathToFileURL = (p: string) => new URL(`file://${p}`);
export default { EOL, release, platform, sep, relative, join, fileURLToPath, pathToFileURL };
