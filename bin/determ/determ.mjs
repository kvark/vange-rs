// Run the wasm build of `determ` under node:
//   node determ.mjs PKG_DIR test X Y QUANTA
//   node determ.mjs PKG_DIR zips COMMON.zip LEVEL.zip X Y QUANTA
// PKG_DIR holds `wasm-bindgen --target nodejs` output of determ.wasm.
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import path from 'node:path';
const require = createRequire(import.meta.url);
const [pkg, mode, ...rest] = process.argv.slice(2);
const m = require(path.resolve(pkg, 'determ.js'));
let out;
if (mode === 'test') {
  const [x, y, q] = rest.map(Number);
  out = m.tumble_test(x, y, q);
} else {
  const [c, l, x, y, q] = rest;
  out = m.tumble_zips(readFileSync(c), readFileSync(l), Number(x), Number(y), Number(q));
}
process.stdout.write(out);
