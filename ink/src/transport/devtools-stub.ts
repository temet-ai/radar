// Stand-in for `react-devtools-core` in the bundle (see the `bundle` script).
// Ink only calls it when DEV=true; the shipped renderer has no devtools.
const stub = {
  initialize(): void {},
  connectToDevTools(): void {},
};
export default stub;
