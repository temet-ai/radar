// Ink's own output.js (aliased as `ink-output`, see fast-output.ts).
declare module "ink-output" {
  export default class Output {
    constructor(options: { width: number; height: number });
    operations: unknown[];
    get(): { output: string; height: number };
  }
}
