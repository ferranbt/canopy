const fs = require("fs");

const mark = process.env.INPUT_MARK;
console.log(`main ${mark}`);

fs.appendFileSync(process.env.GITHUB_STATE, `mark=${mark}\n`);
