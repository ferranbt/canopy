// A JavaScript action with no dependencies: it reads INPUT_* and writes $GITHUB_OUTPUT.
const fs = require("fs");

const who = process.env["INPUT_WHO-TO-GREET"] || "World";
console.log(`Hello, ${who}!`);
console.log(`action_path=${process.env.GITHUB_ACTION_PATH}`);

// Outputs and env vars leave a step through the files the runner points at.
fs.appendFileSync(process.env.GITHUB_OUTPUT, `greeted-at=${who}\n`);
fs.appendFileSync(process.env.GITHUB_ENV, `GREETED_BY=node\n`);

// An action in the repository being run gets no `pre` hook, so there is nothing here from
// one; what this leaves is what the `post` hook reads back.
fs.appendFileSync(process.env.GITHUB_STATE, `greeted=node\n`);
