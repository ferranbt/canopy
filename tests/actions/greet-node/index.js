// A JavaScript action with no dependencies: it reads INPUT_* and writes $GITHUB_OUTPUT.
const fs = require("fs");

const who = process.env["INPUT_WHO-TO-GREET"] || "World";
console.log(`Hello, ${who}! (from node ${process.versions.node})`);
console.log(`action_path=${process.env.GITHUB_ACTION_PATH}`);

// Outputs and env vars leave a step through the files the runner points at.
fs.appendFileSync(process.env.GITHUB_OUTPUT, `greeted-at=${who}\n`);
fs.appendFileSync(process.env.GITHUB_ENV, `GREETED_BY=node\n`);

// What the pre hook left, and what the post hook will read back.
if (process.env.NODE_RAN_PRE !== "yes") {
  console.log("the pre hook did not run first");
  process.exit(1);
}
fs.appendFileSync(process.env.GITHUB_STATE, `greeted=node\n`);
