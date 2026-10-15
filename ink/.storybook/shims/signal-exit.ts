// signal-exit: nothing exits a story.
export default function signalExit() {
  return () => {};
}
export const onExit = signalExit;
