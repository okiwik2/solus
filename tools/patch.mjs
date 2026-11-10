// usage: node tools/patch.mjs <file> <patchfile.json>  where json = [[old,new],...]
import fs from 'fs';
const [,, file, pf] = process.argv;
let s = fs.readFileSync(file, 'utf8');
const pairs = JSON.parse(fs.readFileSync(pf, 'utf8'));
for (const [a, b] of pairs) {
  if (!s.includes(a)) { console.error('NOT FOUND in ' + file + ':\n' + a); process.exit(1); }
  s = s.replace(a, () => b);
}
fs.writeFileSync(file, s);
console.log('patched', file, pairs.length);
