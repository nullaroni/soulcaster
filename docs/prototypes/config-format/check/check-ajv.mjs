// PROTOTYPE: second validator, using ECMA-262 regular expressions as JSON Schema specifies.
// node check-ajv.mjs <ajv install dir> <docs/prototypes/config-format> <corpus from check.py>
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
const [ajvDir, root, corpusPath] = process.argv.slice(2);
const require = createRequire(join(ajvDir, "x.js"));
const Ajv2020 = require("ajv/dist/2020").default;
const schema = JSON.parse(readFileSync(join(root, "config.schema.json"), "utf8"));
const validate = new Ajv2020({ strict: true, allErrors: true }).compile(schema);
// JSON.parse isn't the engine's parser: it keeps the last duplicate key and turns 1e999 into Infinity.
const PARSER_ONLY = new Set(["duplicate-key.json", "duplicate-schema-version.json", "number-overflow.json"]);
let failures = 0;
for (const [dir, wantValid] of [["examples", true], ["examples/invalid", false], ["examples/lint", true]]) {
  let pass = 0, total = 0;
  for (const f of readdirSync(join(root, dir)).filter((n) => n.endsWith(".json"))) {
    if (PARSER_ONLY.has(f)) continue;
    total++;
    let ok = false;
    try { ok = validate(JSON.parse(readFileSync(join(root, dir, f), "utf8"))); } catch { ok = false; }
    if (ok === wantValid) pass++; else { failures++; console.log("UNEXPECTED", dir, f, validate.errors?.[0]?.message); }
  }
  console.log(`${dir}: ${pass}/${total} behaved as expected (${wantValid ? "accepted" : "refused"})`);
}
const corpus = JSON.parse(readFileSync(corpusPath, "utf8"));
let right = 0;
for (const c of corpus.cases) {
  if (validate(c.config) === c.valid) right++; else { failures++; console.log("UNEXPECTED", c.label, validate.errors?.[0]?.message); }
}
console.log(`conformance: ${right}/${corpus.cases.length} behaved as expected`);
console.log(`parser cases: ${corpus.parser.length} skipped, because JSON.parse isn't the engine's parser`);
let verdicts = "";
for (const c of corpus.fuzz) verdicts += validate(c) ? "1" : "0";
const accepted = [...verdicts].filter((v) => v === "1").length;
console.log(`fuzz: ${verdicts.length} cases, ${accepted} accepted, sha256 ${createHash("sha256").update(verdicts).digest("hex")}`);
process.exit(failures ? 1 : 0);
