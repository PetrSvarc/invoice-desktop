import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { IdokladImport } from '../src/IdokladImport';
import { AnnualFo } from '../src/AnnualFo';
import '../src/style.css';

// Isolated visual fixture: no Tauri runtime, network, database, or user documents.
const initialConfig = { year: 2025, expense_mode: 'percentage', default_category: 'trade_60', bank_allocations: [], manual_entries: [], reviewed_purchase_ids: [], reviewed_archive_ids: [], records_complete_fingerprint: null };
let config = { ...initialConfig };
const bank = [
  { id: 'fixture-bank-1', booked: '2025-06-18', amount: '12100', currency: 'CZK', description: 'Úhrada faktury 2025-0021', variable_symbol: '20250021', invoice_id: 'fixture-issued' },
  { id: 'fixture-bank-2', booked: '2025-09-15', amount: '8500', currency: 'CZK', description: 'Platba od zákazníka', variable_symbol: '', invoice_id: null },
  { id: 'fixture-bank-3', booked: '2025-11-02', amount: '-2460', currency: 'CZK', description: 'Materiál', variable_symbol: '', invoice_id: null },
];
const purchases = [{ id: 'fixture-purchase-1', supplier: 'Ukázkový dodavatel', document_number: 'D-104', supply_date: '2025-11-01', currency: 'CZK', base: '2000', vat: '460' }];
const summary = {
  year: 2025, complete: false, records_complete: false, expense_mode: 'percentage', income_czk: '10000', expenses_czk: '6000', section7_difference_czk: '4000',
  percentage_groups: [{ category: 'trade_60', rate: 60, income_czk: '10000', expense_czk: '6000', cap_czk: '1200000' }],
  archived_invoices: [{ invoice_id: 'fixture-archive-1', number: 'ID-2025-73', currency: 'CZK', gross: '3750' }],
  outstanding: [{ invoice_id: 'fixture-issued', number: '2025-0021', currency: 'CZK', gross: '24200', paid_through_year_end: '12100', outstanding: '12100' }],
  rows: [], issues: [{ code: 'bank_unreviewed', message: 'Bankovní pohyb nemá klasifikaci; nelze určit vztah k §7.', source_id: 'fixture-bank-2' }, { code: 'archive_payment_unknown', message: 'Archivní faktura nemá úplnou historii úhrad a DPH; příjmy vyžadují kontrolu.', source_id: 'fixture-archive-1' }, { code: 'records_not_confirmed', message: 'Potvrďte úplnost příjmů a výdajů po kontrole všech podkladů.', source_id: '' }],
};
const importPreview = {
  sha256: 'fixture', source_format: 'iDoklad POHODA issued-invoice XML', total: 3, importable: 1, duplicates: 1, blocked: 1, can_commit: false,
  invoices: [
    { source_id: '123', number: 'ID-2025-73', issue_date: '2025-05-19', customer: 'Kavárna Modrá', gross: '3750', currency: 'CZK', status: 'importable' },
    { source_id: '124', number: 'ID-2025-74', issue_date: '2025-05-20', customer: 'Studio Břeh', gross: '9680', currency: 'CZK', status: 'duplicate', reason: 'Faktura již v archivu existuje.' },
    { source_id: '125', number: '', issue_date: '', customer: '', gross: '', currency: '', status: 'blocked', reason: 'Chybí povinné číslo faktury.' },
  ], warnings: ['Údaje o platbách, režimu DPH a historickém prodejci nejsou v tomto exportu spolehlivě dostupné.'],
};
const ask = async <T,>(action: string, payload: Record<string, unknown> = {}): Promise<T> => {
  if (action === 'idoklad.preview') return importPreview as T;
  if (action === 'idoklad.commit') return { imported: 1, duplicates: 1 } as T;
  if (action === 'annual.config') return { ...config, year: payload.year } as T;
  if (action === 'annual.summary') return summary as T;
  if (action === 'annual.configure') { config = payload.config as typeof config; return config as T; }
  if (action === 'annual.csv') return { content: 'fixture' } as T;
  throw new Error(`Unsupported fixture action: ${action}`);
};
function Fixture() {
  const [tab, setTab] = useState<'annual' | 'import'>('annual');
  useEffect(() => {
    if (tab !== 'import') return;
    const fileInput = document.querySelector<HTMLInputElement>('.import-picker input[type=file]');
    if (!fileInput) return;
    const transfer = new DataTransfer();
    transfer.items.add(new File(['<?xml version="1.0" encoding="UTF-8"?><dataPack/>'], 'iDoklad2PohodaExportIssuedInvoice_fixture.xml', { type: 'application/xml' }));
    fileInput.files = transfer.files;
    fileInput.dispatchEvent(new Event('change', { bubbles: true }));
  }, [tab]);
  return <div className="app"><aside className="sidebar"><div className="brand"><div className="brand-mark">f.</div><div><b>faktura.</b><small>UI FIXTURE</small></div></div><div className="nav-label">PRACOVNÍ PROSTOR</div><nav><button className={`nav-item ${tab === 'annual' ? 'active' : ''}`} onClick={() => setTab('annual')}>Přehled FO</button><button className={`nav-item ${tab === 'import' ? 'active' : ''}`} onClick={() => setTab('import')}>Import z iDokladu</button></nav></aside><div className="main"><header className="topbar"><div className="breadcrumb">Ukázkový prostor <span>/</span> <strong>{tab === 'annual' ? 'Přehled FO' : 'Import z iDokladu'}</strong></div></header><main className="content">{tab === 'annual' ? <AnnualFo ask={ask} bank={bank} purchases={purchases} saveText={async () => {}} revision="fixture"/> : <IdokladImport ask={ask} onImported={async () => {}}/>}</main></div></div>;
}
createRoot(document.getElementById('root')!).render(<Fixture/>);
