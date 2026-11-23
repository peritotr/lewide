#!/usr/bin/env node
// Compares the texts used in the source code with each translation file.
//
//   node scripts/i18n-check.mjs            report; fails on unused entries and on gaps in complete languages
//   node scripts/i18n-check.mjs --list     print every source text, one JSON string per line
//   node scripts/i18n-check.mjs --add tr   add the missing texts to a file with empty translations
//
// A text is found where it is written as a literal in t("…"), tx("…"),
// msg("…") or as the last form given to tn(count, "one", "other").
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const ts = createRequire(import.meta.url)("typescript");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const localeDir = path.join(root, "src/i18n");
/** Languages that must have every text; the others may fall back to English. */
const COMPLETE = ["tr"];

const sources = new Map();
(function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) scan(full);
  }
})(path.join(root, "src"));

function scan(file) {
  const sf = ts.createSourceFile(file, fs.readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true, file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS);
  const visit = (node) => {
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
      const name = node.expression.text;
      const arg = name === "tn" ? node.arguments[2] : ["t", "tx", "msg"].includes(name) ? node.arguments[0] : null;
      if (arg && (ts.isStringLiteral(arg) || ts.isNoSubstitutionTemplateLiteral(arg))) {
        const where = `${path.relative(root, file)}:${sf.getLineAndCharacterOfPosition(arg.getStart()).line + 1}`;
        if (!sources.has(arg.text)) sources.set(arg.text, where);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}

const [mode, target] = process.argv.slice(2);
if (mode === "--list") {
  for (const text of [...sources.keys()].sort()) console.log(JSON.stringify(text));
  process.exit(0);
}

const placeholders = (text) => [...String(text).matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join(",");
let failed = false;
for (const file of fs.readdirSync(localeDir).filter((f) => f.endsWith(".json"))) {
  const code = path.basename(file, ".json");
  const full = path.join(localeDir, file);
  const dictionary = JSON.parse(fs.readFileSync(full, "utf8"));
  const missing = [...sources.keys()].filter((text) => !dictionary[text]);
  const unused = Object.keys(dictionary).filter((text) => !sources.has(text));
  // A translation that drops or invents a placeholder shows a literal {name} or loses a value.
  const broken = Object.entries(dictionary).filter(([text, entry]) =>
    (typeof entry === "string" ? [entry] : Object.values(entry)).some((form) => form && placeholders(form) !== placeholders(text)),
  );

  if (mode === "--add" && target === code) {
    for (const text of missing) dictionary[text] = "";
    const sorted = Object.fromEntries(Object.entries(dictionary).sort(([a], [b]) => a.localeCompare(b, "en")));
    fs.writeFileSync(full, `${JSON.stringify(sorted, null, 2)}\n`);
    console.log(`${file}: added ${missing.length} texts to translate`);
    continue;
  }

  const complete = COMPLETE.includes(code);
  console.log(`${file}: ${sources.size - missing.length}/${sources.size} translated, ${unused.length} unused, ${broken.length} with wrong placeholders`);
  for (const text of missing.slice(0, complete ? 200 : 5)) console.log(`  missing  ${JSON.stringify(text)}  (${sources.get(text)})`);
  for (const text of unused) console.log(`  unused   ${JSON.stringify(text)}`);
  for (const [text] of broken) console.log(`  placeholders differ  ${JSON.stringify(text)}`);
  if (unused.length > 0 || broken.length > 0 || (complete && missing.length > 0)) failed = true;
}
process.exit(failed ? 1 : 0);
