// Runs before any story module: the globals Ink and radar's renderer read at load time.
import process from "./shims/process";
import chalk from "chalk";

const g = globalThis as unknown as { process?: unknown; global?: unknown };
g.process ??= process;
g.global ??= globalThis;
// Ink colours through chalk, which in a browser guesses its level from the user agent. radar's host says the tier
// (the Look's slot words already carry it), so chalk writes every colour it is given.
chalk.level = 3;
