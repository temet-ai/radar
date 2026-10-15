// node:process for the browser. Ink reads the environment, the platform and the standard streams; every story
// passes its own stdout and stdin to render(), so these defaults only have to exist.
import { EventEmitter } from "./events.js";

class NullStream extends EventEmitter {
  isTTY = false;
  columns = 80;
  rows = 24;
  write() {
    return true;
  }
  setRawMode() {}
  setEncoding() {}
  resume() {}
  pause() {}
  ref() {}
  unref() {}
  read() {
    return null;
  }
}

class BrowserProcess extends EventEmitter {
  env: Record<string, string | undefined> = { NODE_ENV: "production" };
  platform = "browser";
  argv: string[] = [];
  execArgv: string[] = [];
  versions: Record<string, string> = {};
  version = "";
  pid = 1;
  exitCode: number | undefined;
  stdout = new NullStream();
  stderr = new NullStream();
  stdin = new NullStream();
  cwd() {
    return "/";
  }
  exit() {}
  nextTick(f: (...args: unknown[]) => void, ...args: unknown[]) {
    queueMicrotask(() => f(...args));
  }
  hrtime = Object.assign(
    (prev?: [number, number]): [number, number] => {
      const t = performance.now();
      const s = Math.floor(t / 1000);
      const ns = Math.floor((t % 1000) * 1e6);
      return prev ? [s - prev[0], ns - prev[1]] : [s, ns];
    },
    { bigint: () => BigInt(Math.floor(performance.now() * 1e6)) },
  );
  emitWarning() {}
  memoryUsage() {
    return { rss: 0, heapTotal: 0, heapUsed: 0, external: 0 };
  }
}

const process = new BrowserProcess();
export default process;
export const env = process.env;
export const platform = process.platform;
export const stdout = process.stdout;
export const stderr = process.stderr;
export const stdin = process.stdin;
export const argv = process.argv;
export const nextTick = process.nextTick.bind(process);
export const cwd = process.cwd.bind(process);
