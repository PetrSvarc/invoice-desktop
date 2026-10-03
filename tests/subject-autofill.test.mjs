import test from 'node:test';
import assert from 'node:assert/strict';
import { createSubjectAutofill } from '../src/subject-autofill.ts';

const pause = () => new Promise(resolve => setTimeout(resolve, 20));
const result = (overrides = {}) => ({
  ico: '27074358', dic: 'CZ27074358', name: 'Company from registry',
  address: 'Registered address', country: 'CZ', source: 'ARES + Registr DPH',
  checked_at: '2026-10-01T12:00:00Z', vat_status: 'Plátce DPH', warnings: [], ...overrides,
});
function form(initial = {}) {
  let value = { ico: '', dic: '', name: '', address: '', country: 'CZ', ...initial };
  let state;
  const requests = [];
  const controller = createSubjectAutofill({
    read: () => value,
    patch: fields => { value = { ...value, ...fields }; },
    publish: next => { state = next; },
    lookup: (kind, identifier) => new Promise((resolve, reject) => requests.push({ kind, identifier, resolve, reject })),
  }, 5);
  return { controller, requests, get value() { return value; }, get state() { return state; },
    manual: fields => { value = { ...value, ...fields }; } };
}

test('typing debounces complete identifiers; filling DIČ does not cause a second lookup', async () => {
  const f = form();
  f.controller.edit('ico', '270');
  await pause();
  assert.equal(f.requests.length, 0);
  f.controller.edit('ico', '11111111');
  f.controller.edit('ico', '27074358');
  await pause();
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].identifier, '27074358');
  f.requests[0].resolve(result());
  await pause();
  assert.equal(f.value.name, 'Company from registry');
  assert.equal(f.value.dic, 'CZ27074358');
  f.controller.sync();
  await pause();
  assert.equal(f.requests.length, 1);
  assert.equal(f.state.phase, 'success');
});

test('an old IČO response cannot overwrite a newer DIČ edit', async () => {
  const f = form({ ico: '11111111' });
  f.controller.search('ico');
  f.controller.edit('dic', 'CZ27074358');
  await pause();
  f.requests[1].resolve(result());
  await pause();
  f.requests[0].resolve(result({ name: 'Stale company', ico: '11111111' }));
  await pause();
  assert.equal(f.value.name, 'Company from registry');
  assert.equal(f.value.ico, '27074358');
});

test('manual edits during the debounce and network request take priority; unrelated fields survive', async () => {
  const f = form({ email: 'private@example.test', bank_account: '123/0100', registrations: [{ regime: 'identified' }] });
  f.controller.edit('ico', '27074358');
  f.manual({ name: 'My preferred name' });
  await pause();
  f.manual({ address: 'My correspondence address' });
  f.requests[0].resolve(result());
  await pause();
  assert.equal(f.value.name, 'My preferred name');
  assert.equal(f.value.address, 'My correspondence address');
  assert.equal(f.value.email, 'private@example.test');
  assert.equal(f.value.bank_account, '123/0100');
  assert.deepEqual(f.value.registrations, [{ regime: 'identified' }]);
  assert.equal(f.state.preservedEdits, true);
});

test('a different subject clears missing old identifiers instead of combining two identities', async () => {
  const f = form({ ico: '11111111', dic: 'CZ11111111', name: 'Old company', address: 'Old address' });
  f.controller.edit('ico', '27074358');
  await pause();
  f.requests[0].resolve(result({ dic: null, address: null }));
  await pause();
  assert.equal(f.value.dic, '');
  assert.equal(f.value.address, '');
  assert.equal(f.value.name, 'Company from registry');
});

test('DIČ-only results never invent an IČO; optional CZ prefix is supported', async () => {
  const f = form({ ico: '11111111' });
  f.controller.edit('dic', '123456789');
  await pause();
  assert.equal(f.requests[0].kind, 'dic');
  f.requests[0].resolve(result({ ico: null, dic: 'CZ123456789' }));
  await pause();
  assert.equal(f.value.ico, '');
  assert.equal(f.value.dic, 'CZ123456789');
});

test('closing the form cancels queued work and ignores in-flight responses', async () => {
  const f = form();
  f.controller.edit('ico', '27074358');
  f.controller.dispose();
  await pause();
  assert.equal(f.requests.length, 0);
  const g = form({ ico: '27074358' });
  g.controller.search('ico');
  g.controller.dispose();
  g.requests[0].resolve(result());
  await pause();
  assert.equal(g.value.name, '');
});

test('registry errors leave the form editable and permit retry', async () => {
  const f = form({ ico: '27074358', name: 'Manual company' });
  f.controller.search('ico');
  f.requests[0].reject(new Error('Registry unavailable'));
  await pause();
  assert.equal(f.state.phase, 'error');
  assert.equal(f.value.name, 'Manual company');
  f.controller.search('ico');
  f.requests[1].resolve(result());
  await pause();
  assert.equal(f.state.phase, 'success');
});

test('opening existing records and entering foreign DIČ do not send automatic requests', async () => {
  const f = form({ ico: '27074358' });
  f.controller.sync();
  await pause();
  assert.equal(f.requests.length, 0);
  f.controller.edit('dic', 'DE123456789');
  await pause();
  assert.equal(f.requests.length, 0);
  assert.match(f.state.message, /pouze české DIČ/);
});

test('refreshing the form invalidates a response for the previously displayed subject', async () => {
  const f = form({ ico: '27074358' });
  f.controller.search('ico');
  f.manual({ ico: '99999999', name: 'Refreshed company' });
  f.controller.sync();
  f.requests[0].resolve(result());
  await pause();
  assert.equal(f.value.name, 'Refreshed company');
  assert.equal(f.state.phase, 'idle');
});
