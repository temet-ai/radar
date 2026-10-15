import "./transport/require-shim.js";
import { restoreFetch } from "./transport/no-fetch.js";
import { run } from "./app/run.js";

restoreFetch();
await run();
