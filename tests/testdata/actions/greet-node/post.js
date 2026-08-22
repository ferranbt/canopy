// A post hook, which reads what the main entry point saved through `$GITHUB_STATE`.
const saved = process.env.STATE_greeted;

if (saved !== "node") {
  console.log(`the post step saw STATE_greeted=${saved}`);
  process.exit(1);
}
console.log("the post step read the state the main step saved");
