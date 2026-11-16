// Host-only boundary: use the reference's ECMAScript number/string/key semantics.
const {createHash} = require('node:crypto');
const fs = require('node:fs');
function sortedJson(value) {
  if (Array.isArray(value)) return value.map(sortedJson);
  if (value === null || typeof value !== 'object') return value;
  const out = {};
  for (const key of Object.keys(value).sort()) out[key] = sortedJson(value[key]);
  return out;
}
const hashes = JSON.parse(fs.readFileSync(0, 'utf8')).map(text => {
  const raw = JSON.parse(text);
  const selected = {name: raw.name, description: raw.description, inputSchema: raw.inputSchema, annotations: raw.annotations};
  return createHash('sha256').update(JSON.stringify(sortedJson(selected))).digest('hex').slice(0,16);
});
process.stdout.write(JSON.stringify(hashes));
