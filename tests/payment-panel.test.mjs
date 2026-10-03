import assert from 'node:assert/strict';
import { mkdir, rm } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { build } from 'esbuild';

const root = resolve(import.meta.dirname, '..');
const output = resolve(root, `.qa-data/payment-panel-test-${process.pid}.mjs`);
await mkdir(resolve(root, '.qa-data'), { recursive: true });
try {
  await build({ entryPoints: [resolve(root, 'src/PaymentPanel.tsx')], outfile: output, bundle: true, platform: 'node', format: 'esm', packages: 'external' });
} catch (error) {
  await rm(output, { force: true });
  throw error;
}
const { PaymentPanel } = await import(pathToFileURL(output).href);
await rm(output, { force: true });

const manual = {
  account_name: 'Archivovaný dodavatel', bank_name: 'Stará banka', bank_account: '111111/0100', iban: 'CZ6508000000192000145399', bic: 'GIBACZPX',
  variable_symbol: '111', constant_symbol: '0308', specific_symbol: '', amount: '199.00', currency: 'CZK',
};
const payment = {
  ...manual, account_name: 'Vydavatel účtu', bank_name: 'Nová banka', bank_account: '222222/0800', variable_symbol: '20260001', specific_symbol: '42',
  amount: '121.00', method: 'bank_transfer', due_date: '2026-10-14', qr_svg: '<svg xmlns="http://www.w3.org/2000/svg"/>', qr_payload: 'SPD*1.0', qr_error: null,
};
const render = (value, error = '') => renderToStaticMarkup(createElement(PaymentPanel, {
  payment: value, manual, pending: false, error, qrStatus: 'ready', onQrLoad: () => {}, onQrError: () => {},
}));

test('payment panel shows the issued payment response and its QR without mixing in draft details', () => {
  const html = render(payment);
  assert.match(html, /Vydavatel účtu/);
  assert.match(html, /222222\/0800/);
  assert.match(html, /20260001/);
  assert.match(html, /Specifický symbol<\/span><strong>42<\/strong>/);
  assert.match(html, /121,00/);
  assert.match(html, /alt="QR Platba pro bankovní převod"/);
  assert.doesNotMatch(html, /Archivovaný dodavatel|111111\/0100|199,00/);
});

test('QR refusal leaves account, amount, symbols, and a useful error visible', () => {
  const html = render({ ...payment, qr_svg: null, qr_payload: null, qr_error: 'QR Platba je dostupná pouze pro CZK.' });
  assert.match(html, /222222\/0800/);
  assert.match(html, /20260001/);
  assert.match(html, /121,00/);
  assert.match(html, /QR Platba je dostupná pouze pro CZK/);
  assert.doesNotMatch(html, /<img/);
});

test('failed payment fetch falls back to the archived seller account', () => {
  const html = render(null, 'Platební údaje se nepodařilo načíst.');
  assert.match(html, /Archivovaný dodavatel/);
  assert.match(html, /111111\/0100/);
  assert.match(html, /199,00/);
  assert.match(html, /Platební údaje se nepodařilo načíst/);
  assert.doesNotMatch(html, /<img/);
});
