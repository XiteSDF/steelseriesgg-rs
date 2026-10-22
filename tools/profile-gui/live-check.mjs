// live-check.mjs - the macro editor end to end, against a running `ssgg gui`.
//
// The editor windows talk to the server only through `POST /api/macros/session`, and the fake DOM of
// selftest.mjs cannot open a window. This script runs the parts that are pure HTTP: the profile the
// editor serializes, the numbers the codec answers with, and the two refusals the codec makes - a record
// list past the block, and an edited list, which the write refuses because the block carries GG's own
// structures between the records. Run it against a live gui:
//
//   ssgg gui -p 0 &
//   SSGG_GUI_URL=http://127.0.0.1:<port> SSGG_GUI_TOKEN=<token from the page's meta tag> \
//     node tools/profile-gui/live-check.mjs
//
// Without the two environment variables it says so and exits 0; selftest.mjs keeps the same checks
// skipped in the suite.
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import { installFakeDom, loadApp, loadClassic } from './fake-dom.mjs';

const url = (process.env.SSGG_GUI_URL ?? '').replace(/\/$/, '');
const token = process.env.SSGG_GUI_TOKEN ?? '';
if (!url || !token) {
  console.log('live-check: set SSGG_GUI_URL and SSGG_GUI_TOKEN (the token in the page meta tag) and try again.');
  process.exit(0);
}

vm.runInThisContext(readFileSync(`${import.meta.dirname}/products.generated.js`, 'utf8'), { filename: 'products.generated.js' });
installFakeDom();
const app = loadApp(import.meta.dirname);
loadClassic('macros.js');
const macro = globalThis.SsggMacroEditor;

let failed = 0;
const check = (label, condition, detail) => {
  console.log(`  ${condition ? 'ok  ' : 'FAIL'} ${label}${condition ? '' : ` - ${detail}`}`);
  if (!condition) failed += 1;
};

const call = async (method, body) => {
  const res = await fetch(`${url}/api/macros/session`, {
    method,
    headers: {
      origin: new URL(url).origin,
      'x-ssgg-token': token,
      ...(body ? { 'content-type': 'application/json' } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  return { status: res.status, json: text ? JSON.parse(text) : null, text };
};

const record = (id, keys) => ({
  mode: 4,
  id,
  flags: macro.RECORD_FLAGS,
  interval_ms: 100,
  actions: keys.flatMap((key) => [macro.makePress(key), macro.makeRelease(key)]),
});

// The editor keeps one file open, so opening the sample makes it the active document, as in the page.
const openSample = () => app.addFile('live.json', readFileSync(`${import.meta.dirname}/examples/sample-identity.json`, 'utf8')).file;
const serialized = () => JSON.parse(app.serializeDoc(app.activeDoc()));

console.log('the macro editor session against a live ssgg gui');

// 1. the profile as the read reported it: the codec answers with the numbers of its own block, and the
// session hands the same profile back to the editor window.
openSample();
const asRead = serialized();
const save = await call('POST', { profile: asRead });
const read = await call('GET', null);
const region = save.json?.region ?? {};
check(
  "the profile as it was read is accepted, and the block numbers are the codec's",
  save.status === 200 && save.json.ok === true
    && Number.isInteger(save.json.revision)
    && region.records === (asRead.macros ?? []).length
    && region.used_bytes + region.free_bytes === region.len && region.len === macro.REGION_LEN_B
    && region.used_bytes === (asRead.macros ?? []).reduce((sum, r) => sum + r.record_bytes, 0)
    && read.status === 200 && read.json.revision === save.json.revision
    && JSON.stringify(read.json.region) === JSON.stringify(region)
    && JSON.stringify(read.json.profile.macros) === JSON.stringify(asRead.macros),
  `save ${save.status} ${JSON.stringify(save.json).slice(0, 220)} get ${read.status} ${read.text.slice(0, 160)}`,
);

// 2. an edited list: the editor adopts it into the open document, and the write path refuses it, because
// the block between the records is GG's own. The refusal is the codec's, in its words, at the HTTP layer.
openSample();
const edited = app.adoptMacroRecords([record(1, [0x22, 0x26]), record(2, [0x25, 0x26, 0x22])]);
const editedProfile = serialized();
const refused = await call('POST', { profile: editedProfile });
check(
  'an edited record list is refused by the codec, in the editor and on the wire',
  edited.ok === true && edited.used === 56 && edited.records === 2
    && refused.status === 422
    && String(refused.json.error).includes('the record list does not match the macro block')
    && String(refused.json.error).includes('GG'),
  `adopted ${JSON.stringify(edited)} wire ${refused.status} ${refused.text.slice(0, 220)}`,
);

// 3. a list past the block: packed so the JSON fields are the fields the codec reads, posted straight to
// the wire - the editor would never post it, and the codec is the one that says how many bytes are too many.
const bigRecords = macro.packRecords(
  Array.from({ length: 86 }, (unused, position) => record(position + 1, Array.from({ length: 13 }, (u, key) => 0x04 + key))),
  macro.regionOf(app.activeDoc()),
);
const needed = 86 * (macro.HEADER_LEN + 26 * macro.ACTION_LEN);
const big = await call('POST', { profile: { ...serialized(), macros: bigRecords } });
check(
  'a record list past the block is refused with the codec numbers',
  big.status === 413 && String(big.json.error).includes('macro records need')
    && String(big.json.error).includes(`${needed}`) && String(big.json.error).includes(`has ${region.len}`),
  `wire ${big.status} ${big.text.slice(0, 220)} (needed ${needed} of ${region.len})`,
);

// 4. the channel's access rules.
const noToken = await fetch(`${url}/api/macros/session`, {
  method: 'POST',
  headers: { origin: new URL(url).origin, 'content-type': 'application/json' },
  body: JSON.stringify({ profile: asRead }),
});
const foreign = await fetch(`${url}/api/macros/session`, {
  method: 'POST',
  headers: { origin: 'http://127.0.0.1:9', 'x-ssgg-token': token, 'content-type': 'application/json' },
  body: JSON.stringify({ profile: asRead }),
});
const pageForeign = await fetch(`${url}/macros.html`, { headers: { origin: 'http://127.0.0.1:9' } });
check(
  'no token, a foreign origin, and the editor page from a foreign origin all answer 403',
  noToken.status === 403 && foreign.status === 403 && pageForeign.status === 403,
  `no token ${noToken.status}, foreign origin ${foreign.status}, page ${pageForeign.status}`,
);

console.log('');
console.log(failed === 0 ? 'live-check: all checks passed' : `live-check: ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
