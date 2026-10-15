// node:module for the browser: stack-utils (behind Ink's error overview) reads `builtinModules` when it loads.
export const builtinModules: string[] = [];
export function createRequire(): (id: string) => never {
  return (id: string) => {
    throw new Error(`require(${id}) is not available in Storybook`);
  };
}
export default { builtinModules, createRequire };
