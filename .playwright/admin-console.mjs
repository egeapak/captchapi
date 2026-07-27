// End-to-end check of the admin console at /admin.
//
// Drives the real page against a real server: unlocks it, edits live fields, applies them,
// and verifies the change through a separate API call rather than by reading the DOM back.
// The point is that the console and the server agree, which no Rust test can assert — the
// Rust tests cover the routes and the CSP, but nothing there runs the JavaScript.
//
// The server must already be running, built with the `admin-ui` feature. See README.md here.
//
//   BASE_URL             default http://127.0.0.1:3000
//   MASTER_KEY           required — the key the console asks for
//   SHOT_DIR             where PNGs are written, default ./shots next to this file
//   PLAYWRIGHT_CHROMIUM  explicit chromium binary, for images that pre-stage one

import { chromium } from 'playwright';
import { mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const BASE = process.env.BASE_URL ?? 'http://127.0.0.1:3000';
const KEY = process.env.MASTER_KEY;
const OUT = process.env.SHOT_DIR ?? resolve(dirname(fileURLToPath(import.meta.url)), 'shots');

if (!KEY) {
  console.error('MASTER_KEY is required; it is the key the console asks for.');
  process.exit(2);
}
mkdirSync(OUT, { recursive: true });

let pass = 0, fail = 0;
const ok = (name, cond, extra = '') => {
  if (cond) { pass++; console.log(`  PASS  ${name}`); }
  else { fail++; console.log(`  FAIL  ${name} ${extra}`); }
};

// The server's own view of a field, fetched outside the page, so "the console said so" and
// "the server did it" cannot be mistaken for each other.
const server = async (field) => {
  const r = await fetch(`${BASE}/api/v1/admin/config`, { headers: { authorization: 'Bearer ' + KEY } });
  return (await r.json()).config[field].value;
};

const launch = { args: ['--no-sandbox'] };
if (process.env.PLAYWRIGHT_CHROMIUM) launch.executablePath = process.env.PLAYWRIGHT_CHROMIUM;
const browser = await chromium.launch(launch);

const ctx = await browser.newContext({ viewport: { width: 1000, height: 900 }, deviceScaleFactor: 2, colorScheme: 'dark' });
const p = await ctx.newPage();

const errors = [];
// The wrong-key step below deliberately provokes 401s, and the browser logs every non-2xx
// fetch as a console error. Expect exactly as many as unlocking makes rather than ignoring
// 401s in general, so a real auth failure anywhere else still fails the run.
//
// Two, because unlocking fetches the configuration and the store together.
const UNLOCK_REQUESTS = 2;
let expect401 = 0;
const expected = (t) => t.includes('401') && expect401-- > 0;
p.on('console', m => { if (m.type() === 'error' && !expected(m.text())) errors.push(m.text()); });
p.on('pageerror', e => errors.push(String(e)));
p.on('requestfailed', r => errors.push(`${r.url()} ${r.failure()?.errorText}`));

console.log('\n-- gate --');
await p.goto(`${BASE}/admin`, { waitUntil: 'networkidle' });
ok('gate visible', await p.isVisible('#gate'));
ok('panel hidden', !(await p.isVisible('#panel')));
ok('no message on first load', !(await p.isVisible('#msg')));
ok('input is labelled for a11y', (await p.getAttribute('#key', 'aria-label')) === 'Master API key');
await p.screenshot({ path: `${OUT}/gate-dark.png` });

console.log('\n-- wrong key is rejected --');
expect401 = UNLOCK_REQUESTS;
await p.fill('#key', 'not-the-master-key');
await p.click('#gate button');
await p.waitForSelector('#msg.err', { timeout: 5000 });
ok('error shown', (await p.textContent('#msg')).length > 0);
ok('still gated', (await p.isVisible('#gate')) && !(await p.isVisible('#panel')));
await p.screenshot({ path: `${OUT}/gate-error-dark.png` });

console.log('\n-- unlock --');
await p.fill('#key', KEY);
await p.click('#gate button');
await p.waitForSelector('#rows tr', { timeout: 5000 });
ok('panel visible', await p.isVisible('#panel'));
ok('gate hidden', !(await p.isVisible('#gate')));
ok('every parameter is listed', (await p.locator('#rows tr').count()) >= 20);
// Not a fixed count: how many fields are editable depends on how this server was started,
// which is the whole point of the pinning check below. The strict comparison against the
// API's own answer happens there.
const editableCount = await p.locator('#rows input:not([disabled])').count();
ok('at least one field is editable', editableCount > 0, `${editableCount} editable`);
ok('no success toast after unlock', !(await p.isVisible('#msg')));
ok('overrides line hidden when empty', !(await p.isVisible('#ovr')));
await p.screenshot({ path: `${OUT}/console-dark.png`, fullPage: true });

console.log('\n-- every row explains itself --');
const rowCount = await p.locator('#rows tr').count();
const described = await p.locator('#rows .about').count();
ok('one description per row', described === rowCount, `${described} of ${rowCount}`);
const sentences = await p.locator('#rows .about').allTextContents();
ok('descriptions are sentences', sentences.every((s) => s.trim().endsWith('.')));

console.log('\n-- the console offers exactly what the API will accept --');
// The server is the authority on editability; a row the API would refuse must not present an
// enabled input, or an operator types a change that comes back as an error.
const api = await (await fetch(`${BASE}/api/v1/admin/config`, { headers: { authorization: 'Bearer ' + KEY } })).json();
const mismatched = [];
for (const [field, entry] of Object.entries(api.config)) {
  const enabled = await p.locator(`#rows tr:has(td.k:text-is("${field}")) input:not([disabled])`).count();
  if (Boolean(enabled) !== entry.editable) mismatched.push(`${field}: api=${entry.editable} ui=${Boolean(enabled)}`);
}
ok('enabled inputs match the API exactly', mismatched.length === 0, mismatched.join('; '));
ok('every row reports a source', Object.values(api.config).every((e) => typeof e.source === 'string'));
const pinnedFields = Object.entries(api.config).filter(([, e]) => e.reloadable && !e.editable);
ok('pinned rows are tagged with where they were set',
  (await p.locator('#rows .tag.pinned').count()) === pinnedFields.length,
  `${await p.locator('#rows .tag.pinned').count()} tags for ${pinnedFields.length} pinned fields`);

if (pinnedFields.length) {
  const [field] = pinnedFields[0];
  const res = await fetch(`${BASE}/api/v1/admin/config`, {
    method: 'PATCH',
    headers: { 'content-type': 'application/json', authorization: 'Bearer ' + KEY },
    body: JSON.stringify({ [field]: '99' }),
  });
  ok(`patching pinned ${field} is refused with 409`, res.status === 409, `got ${res.status}`);
  ok('refusal carries the config_pinned code', (await res.json()).error === 'config_pinned');
} else {
  // Not a pass. Say so, rather than letting a green run imply coverage it did not have.
  console.log('  SKIP  pinned-field refusal: no field on this server is set by cli/env');
  console.log('        re-run with e.g. CAPTCHA_COMPRESSION=70 in the server\'s environment');
}

console.log('\n-- the config store --');
const storeBefore = await (await fetch(`${BASE}/api/v1/admin/config/stored`, { headers: { authorization: 'Bearer ' + KEY } })).json();
ok('stored endpoint answers', typeof storeBefore.stored === 'object');
const storable = Object.entries(api.config).filter(([, e]) => e.storable && e.editable);
ok('something is storable and editable', storable.length > 0);

if (storable.length) {
  const [field] = storable[0];
  const inputFor = (f) => p.locator(`#rows tr:has(td.k:text-is("${f}")) input`);
  await inputFor(field).fill('33');
  ok('Apply & store is enabled by an edit', await p.isEnabled('#persist'));
  await p.click('#persist');
  await p.waitForSelector('#msg.ok', { timeout: 5000 });

  const after = await (await fetch(`${BASE}/api/v1/admin/config/stored`, { headers: { authorization: 'Bearer ' + KEY } })).json();
  ok(`storing ${field} reaches the database`, after.stored[field] === '33', JSON.stringify(after.stored));
  ok('the row now shows a stored note',
    (await p.locator(`#rows tr:has(td.k:text-is("${field}")) .stored`).count()) === 1);

  // Clean up so the run is repeatable against the same server.
  const del = await fetch(`${BASE}/api/v1/admin/config/stored/${field}`, {
    method: 'DELETE', headers: { authorization: 'Bearer ' + KEY },
  });
  ok('the stored value can be removed again', del.ok);
  await p.click('#reload');
  await p.waitForTimeout(300);
} else {
  console.log('  SKIP  storing a value: nothing on this server is both storable and editable');
}

console.log('\n-- restart control --');
// Only offered when there is something for it to apply, so it never reads as a general
// "bounce the server" button.
const pendingNow = (await (await fetch(`${BASE}/api/v1/admin/config`, { headers: { authorization: 'Bearer ' + KEY } })).json()).pending_restart;
ok('restart button matches whether anything is pending',
  (await p.isVisible('#restart')) === pendingNow.length > 0,
  `pending=${JSON.stringify(pendingNow)}`);
ok('pending banner matches too', (await p.isVisible('#pending')) === pendingNow.length > 0);

console.log('\n-- the three scope classes are visually distinct --');
const colourOf = (sel) => p.locator(sel).first().evaluate((el) => {
  const s = getComputedStyle(el);
  return { fg: s.color, bg: s.backgroundColor };
});
const live = await colourOf('#rows tr.live .tag');
const boot = await colourOf('#rows tr.boot .tag');
const secret = await colourOf('#rows tr.secret .tag');
ok('boot differs from secret', boot.fg !== secret.fg && boot.bg !== secret.bg);
ok('boot differs from live', boot.fg !== live.fg);
const [r, g, bl] = boot.fg.match(/\d+/g).map(Number);
ok('boot foreground is blue-dominant', bl > r && bl > g, `rgb(${r},${g},${bl})`);

console.log('\n-- dirty state --');
// The first field the API says is editable, so the run adapts to how the server was started.
const [editableField] = Object.entries(api.config).find(([, e]) => e.editable);
const before = await server(editableField);
const inputs = p.locator('#rows input:not([disabled])');
await inputs.nth(0).fill('55');
await inputs.nth(1).fill('7');
ok('two modified tags', (await p.locator('#rows .tag.dirty').count()) === 2);
ok('pending count reads 2', (await p.textContent('#pend')).includes('2 unsaved edits'));
ok('apply enabled', await p.isEnabled('#apply'));
await p.screenshot({ path: `${OUT}/console-dirty.png`, fullPage: true });

console.log('\n-- revert --');
await p.click('#revert');
ok('no modified tags after revert', (await p.locator('#rows .tag.dirty').count()) === 0);
ok('apply disabled again', !(await p.isEnabled('#apply')));
ok('server untouched by revert', (await server(editableField)) === before);

console.log('\n-- apply actually changes the server --');
await inputs.nth(0).fill('55');
await p.click('#apply');
await p.waitForSelector('#msg.ok', { timeout: 5000 });
ok(`server value for ${editableField} changed to 55`, (await server(editableField)) === '55');
ok('apply disabled after save', !(await p.isEnabled('#apply')));
ok('overrides line now visible', await p.isVisible('#ovr'));
ok('overrides names the field', (await p.textContent('#ovr')).includes(editableField));
await p.screenshot({ path: `${OUT}/console-overrides.png`, fullPage: true });

console.log('\n-- boot and secret fields are read-only --');
ok('boot input disabled', await p.locator('#rows tr.boot input').first().isDisabled());
ok('secret input disabled', await p.locator('#rows tr.secret input').first().isDisabled());
ok('secret value is redacted',
  (await p.locator('#rows tr.secret input').first().inputValue()).includes('redacted'));

console.log('\n-- reload discards the override --');
await p.click('#reload');
await p.waitForFunction(() => !document.getElementById('ovr').textContent, null, { timeout: 5000 });
ok('server back to its configured value', (await server(editableField)) === before);
ok('overrides line hidden again', !(await p.isVisible('#ovr')));

console.log('\n-- cleanup --');
await p.click('#cleanup');
await p.waitForSelector('#msg.ok', { timeout: 5000 });
ok('cleanup reported', (await p.textContent('#msg')).length > 0);

console.log('\n-- lock --');
await p.click('#lock');
ok('back to the gate', (await p.isVisible('#gate')) && !(await p.isVisible('#panel')));
ok('key field cleared', (await p.inputValue('#key')) === '');

console.log('\n-- light theme --');
const lctx = await browser.newContext({ viewport: { width: 1000, height: 900 }, deviceScaleFactor: 2, colorScheme: 'light' });
const lp = await lctx.newPage();
lp.on('pageerror', (e) => errors.push(String(e)));
await lp.goto(`${BASE}/admin`, { waitUntil: 'networkidle' });
await lp.screenshot({ path: `${OUT}/gate-light.png` });
await lp.fill('#key', KEY);
await lp.click('#gate button');
await lp.waitForSelector('#rows tr');
await lp.screenshot({ path: `${OUT}/console-light.png`, fullPage: true });
const lboot = await lp.locator('#rows tr.boot .tag').first().evaluate((el) => getComputedStyle(el).color);
const lsecret = await lp.locator('#rows tr.secret .tag').first().evaluate((el) => getComputedStyle(el).color);
ok('light: boot differs from secret', lboot !== lsecret, `${lboot} vs ${lsecret}`);

console.log('\n-- narrow viewport --');
const nctx = await browser.newContext({ viewport: { width: 420, height: 900 }, deviceScaleFactor: 2, colorScheme: 'dark' });
const np = await nctx.newPage();
np.on('pageerror', (e) => errors.push(String(e)));
await np.goto(`${BASE}/admin`, { waitUntil: 'networkidle' });
await np.fill('#key', KEY);
await np.click('#gate button');
await np.waitForSelector('#rows tr');
const scroll = await np.evaluate(() => ({ s: document.documentElement.scrollWidth, c: document.documentElement.clientWidth }));
ok('no horizontal scroll at 420px', scroll.s === scroll.c, JSON.stringify(scroll));
await np.screenshot({ path: `${OUT}/console-narrow.png`, fullPage: true });

console.log('\n-- page health --');
ok('zero console, page and request errors', errors.length === 0, JSON.stringify(errors));

await browser.close();
console.log(`\n${pass} passed, ${fail} failed`);
console.log(`screenshots in ${OUT}`);
process.exit(fail ? 1 : 0);
