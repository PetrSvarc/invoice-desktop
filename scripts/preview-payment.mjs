import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { build } from 'esbuild';

const root = resolve(import.meta.dirname, '..');
const qaDir = resolve(root, '.qa-data');
const input = resolve(root, process.argv[2] || '.qa-data/payment-sample.json');
const output = resolve(root, process.argv[3] || '.qa-data/payment-preview.html');
await mkdir(qaDir, { recursive: true });
const sample = JSON.parse(await readFile(input, 'utf8'));
const payment = sample.payment === null ? null : sample.payment || sample;

const compiled = resolve(qaDir, 'invoice-sheet-preview.mjs');
await build({ entryPoints: [resolve(root, 'src/InvoiceSheet.tsx')], outfile: compiled, bundle: true, platform: 'node', format: 'esm', packages: 'external' });
const { InvoiceSheet } = await import(pathToFileURL(compiled).href + `?t=${Date.now()}`);
if (!sample.invoice) throw new Error('Preview sample must contain { invoice, breakdown, payment }');
const sheet = renderToStaticMarkup(createElement(InvoiceSheet, {
  invoice: sample.invoice,
  breakdown: sample.breakdown || [],
  payment,
  paymentPending: false,
  paymentError: sample.error || '',
  qrStatus: 'ready',
  onQrLoad: () => {},
  onQrError: () => {},
}));
const css = await readFile(resolve(root, 'src/style.css'), 'utf8');
const html = `<!doctype html><html lang="cs"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Náhled faktury a QR Platby</title><style>${css}</style></head><body><div class="app"><div class="print-backdrop">${sheet}</div></div></body></html>`;
await writeFile(output, html);
console.log(output);
