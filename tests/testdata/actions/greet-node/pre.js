// A pre hook: it runs before the step it belongs to, so the main entry point can tell.
const fs = require("fs");

fs.appendFileSync(process.env.GITHUB_ENV, "NODE_RAN_PRE=yes\n");
