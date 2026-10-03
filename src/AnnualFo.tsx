import { useEffect, useRef, useState } from 'react';
import { CircleAlert, Plus, ShieldCheck, X } from 'lucide-react';
import { dateLabel, money } from './invoice-format';

type Ask = <T,>(action: string, payload?: Record<string, unknown>) => Promise<T>;
type BankTx = { id: string; booked: string; amount: string; currency: string; description: string; variable_symbol: string; invoice_id: string | null };
type Purchase = { id: string; supplier: string; document_number: string; supply_date: string; currency: string; base: string; vat: string };
type Allocation = { bank_id: string; kind: 'income' | 'income_refund' | 'expense' | 'exclude'; amount: string; category?: string; taxable_czk?: string; note?: string; purchase_id?: string };
type ManualEntry = { id: string; date: string; kind: 'income' | 'income_refund' | 'expense'; category: string; amount: string; currency: string; taxable_czk: string; description: string; evidence: string; purchase_id?: string; fx_note?: string };
type Config = { year: number; expense_mode: 'unselected' | 'percentage' | 'actual'; default_category: string | null; bank_allocations: Allocation[]; manual_entries: ManualEntry[]; records_complete_fingerprint?: string | null; reviewed_archive_ids?: string[]; reviewed_purchase_ids?: string[]; nonlinked_receivables_reviewed?: boolean; confirm_records_complete?: boolean };
type Issue = { code: string; message: string; source_id?: string };
type Summary = { year: number; complete: boolean; records_complete?: boolean; expense_mode: string; income_czk: string; expenses_czk: string; section7_difference_czk: string; percentage_groups: { category: string; rate: number; income_czk: string; expense_czk: string; cap_czk: string }[]; outstanding: { invoice_id: string; number: string; currency: string; gross: string; paid_through_year_end: string; outstanding: string }[]; archived_invoices: { invoice_id?: string; number: string; currency: string; gross: string }[]; rows: Record<string, unknown>[]; issues: Issue[] };
const categories = [
  ['trade_80', '80 % · zemědělství a řemeslné živnosti'],
  ['trade_60', '60 % · ostatní živnosti'],
  ['other_40', '40 % · jiné samostatné činnosti'],
  ['rent_30', '30 % · nájem obchodního majetku'],
] as const;
const categoryLabel = (value: string) => categories.find(([key]) => key === value)?.[1] || value;
const blankEntry = (year: number, category: string): ManualEntry => ({ id: crypto.randomUUID(), date: `${year}-01-01`, kind: 'income', category, amount: '', currency: 'CZK', taxable_czk: '', description: '', evidence: '', purchase_id: '' });

export function AnnualFo({ ask, bank, purchases, saveText, revision }: { ask: Ask; bank: BankTx[]; purchases: Purchase[]; saveText: (filename: string, content: string) => Promise<void>; revision: string }) {
  const [year, setYear] = useState(new Date().getFullYear() - 1);
  const [config, setConfig] = useState<Config | null>(null);
  const [summary, setSummary] = useState<Summary | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [confirmRecords, setConfirmRecords] = useState(false);
  const request = useRef(0);

  useEffect(() => {
    const current = ++request.current;
    setConfig(null);
    setSummary(null);
    setDirty(false);
    setConfirmRecords(false);
    setError('');
    setLoading(true);
    Promise.all([ask<Config>('annual.config', { year }), ask<Summary>('annual.summary', { year })])
      .then(([newConfig, newSummary]) => { if (current === request.current) { setConfig(newConfig); setSummary(newSummary); } })
      .catch(e => { if (current === request.current) setError(String(e)); })
      .finally(() => { if (current === request.current) setLoading(false); });
    return () => { request.current++; };
  }, [ask, year, revision]);

  const update = (next: Config) => { if (saving) return; setConfig(next); setDirty(true); setNotice(''); };
  const setAllocation = (bankId: string, index: number, patch: Partial<Allocation>) => {
    if (!config) return;
    const entries = [...config.bank_allocations];
    const matching = entries.map((a, i) => a.bank_id === bankId ? i : -1).filter(i => i >= 0);
    const target = matching[index];
    if (target === undefined) return;
    entries[target] = { ...entries[target], ...patch };
    update({ ...config, bank_allocations: entries });
  };
  const addAllocation = (tx: BankTx) => {
    if (!config) return;
    const existing = config.bank_allocations.filter(a => a.bank_id === tx.id);
    const amount = existing.length ? '' : String(Math.abs(Number(tx.amount)));
    update({ ...config, bank_allocations: [...config.bank_allocations, { bank_id: tx.id, kind: Number(tx.amount) >= 0 ? 'income' : 'expense', amount, category: config.default_category || '', taxable_czk: '', note: '', purchase_id: '' }] });
  };
  const removeAllocation = (bankId: string, index: number) => {
    if (!config) return;
    let seen = -1;
    update({ ...config, bank_allocations: config.bank_allocations.filter(a => a.bank_id !== bankId || ++seen !== index) });
  };
  const saveConfig = async () => {
    if (!config || saving) return;
    const current = request.current;
    setSaving(true); setError(''); setNotice('');
    try {
      const saved = await ask<Config>('annual.configure', { year, config: { ...config, ...(confirmRecords ? { confirm_records_complete: true } : {}) } });
      const updated = await ask<Summary>('annual.summary', { year });
      if (current !== request.current) return;
      setConfig(saved); setSummary(updated); setDirty(false); setConfirmRecords(false); setNotice('Rozdělení a nastavení roku uloženo.');
    } catch (e) { if (current === request.current) setError(String(e)); }
    finally { setSaving(false); }
  };
  const exportCsv = async () => {
    setError('');
    try {
      const result = await ask<{ content: string }>('annual.csv', { year });
      await saveText(`prehled-fo-${year}.csv`, result.content);
    } catch (e) { setError(String(e)); }
  };

  return <>
    <div className="page-heading"><div><div className="eyebrow">DAŇOVÉ PODKLADY</div><h1>Roční přehled FO</h1><p>Pracovní podklad pro přiznání k dani z příjmů fyzických osob podle § 7.</p></div></div>
    <div className="tax-period annual-period"><label className="field"><span>Kalendářní rok</span><input type="number" min="2000" max={new Date().getFullYear()} value={year} disabled={saving} onChange={e => setYear(Number(e.target.value))}/></label><div className="tax-period-actions"><button className="secondary" disabled={loading || saving || dirty || !summary} onClick={exportCsv}>Exportovat kontrolní CSV</button></div></div>
    {loading && <div className="hint" role="status">Načítám roční podklad…</div>}
    {error && <div className="issue" role="alert"><CircleAlert size={17}/>{error}</div>}
    {notice && <div className="ok-line" role="status"><ShieldCheck size={17}/>{notice}</div>}
    {dirty && <div className="hint annual-stale">Níže uvedené částky odpovídají poslednímu uložení. Uložte změny a přepočítejte rok před exportem.</div>}
    {summary && <>
      <div className={`annual-readiness ${summary.complete ? 'complete' : 'incomplete'}`} role="status"><CircleAlert size={20}/><div><strong>{summary.complete ? 'Podklad je úplný podle evidovaných údajů' : 'Podklad je neúplný · částky jsou pracovní'}</strong><span>{summary.complete ? 'Před podáním porovnejte s účetní evidencí a doklady.' : 'Doplňte klasifikaci plateb, chybějící hotovostní položky a další body níže. CSV zůstává dostupné ke kontrole.'}</span></div></div>
      <div className="stats tax-stats annual-stats"><div className="stat"><span>Daňové příjmy · podle úhrad</span><strong>{money(summary.income_czk)}</strong><small>včetně ručně doložených příjmů</small></div><div className="stat"><span>Výdaje · zvolený režim</span><strong>{money(summary.expenses_czk)}</strong><small>{summary.expense_mode === 'percentage' ? 'výdajový paušál' : summary.expense_mode === 'actual' ? 'skutečné výdaje' : 'režim neurčen'}</small></div><div className="stat accent"><span>Rozdíl příjmů a výdajů</span><strong>{money(summary.section7_difference_czk)}</strong><small>pracovní částka pro § 7</small></div></div>
      <section className="panel padded annual-issues"><h2>Úplnost podkladu</h2>{summary.issues.length ? summary.issues.map((issue, i) => <div className="issue" key={`${issue.code}-${issue.source_id || i}`}><CircleAlert size={17}/><span>{issue.message}</span></div>) : <div className="ok-line"><ShieldCheck size={17}/>V evidovaných datech nejsou otevřené položky.</div>}</section>
      {summary.percentage_groups.length > 0 && <section className="panel table-panel"><div className="panel-head"><h2>Skupiny výdajového paušálu</h2></div><table><thead><tr><th>Druh činnosti</th><th>Příjmy</th><th>Sazba</th><th>Výdaje</th><th>Limit</th></tr></thead><tbody>{summary.percentage_groups.map(group => <tr key={group.category}><td className="strong">{categoryLabel(group.category)}</td><td>{money(group.income_czk)}</td><td>{group.rate} %</td><td>{money(group.expense_czk)}</td><td>{money(group.cap_czk)}</td></tr>)}</tbody></table></section>}
      {!!summary.archived_invoices.length && <section className="panel padded annual-archive"><h2>Importované vystavené faktury</h2><p>Tyto doklady potvrzují vystavení, nikoli příjem peněz. U každého označte, že jste samostatně ověřili úhrady v tomto roce a všechny příjmy správně zařadili.</p><div className="annual-archive-list">{summary.archived_invoices.map((invoice, index) => <label className="annual-archive-item" key={invoice.invoice_id || index}><input type="checkbox" checked={!!invoice.invoice_id && !!config?.reviewed_archive_ids?.includes(invoice.invoice_id)} disabled={!config || !invoice.invoice_id} onChange={e => { if (!config || !invoice.invoice_id) return; const ids = config.reviewed_archive_ids || []; update({ ...config, reviewed_archive_ids: e.target.checked ? [...ids, invoice.invoice_id] : ids.filter(id => id !== invoice.invoice_id) }); }}/><span>{invoice.number || 'Bez čísla'} · {money(invoice.gross, invoice.currency)}</span></label>)}</div></section>}
      {!!summary.outstanding.length && <section className="panel table-panel"><div className="panel-head"><div><h2>Zůstatek podle propojených bankovních úhrad</h2><p>Hotovostní a nepropojené úhrady se zde neodečítají; zkontrolujte je samostatně.</p></div></div><table><thead><tr><th>Číslo</th><th>Fakturováno</th><th>Propojeno do 31. 12.</th><th>Zbývá dle propojení</th></tr></thead><tbody>{summary.outstanding.map(invoice => <tr key={invoice.invoice_id}><td className="strong">{invoice.number}</td><td>{money(invoice.gross, invoice.currency)}</td><td>{money(invoice.paid_through_year_end, invoice.currency)}</td><td>{money(invoice.outstanding, invoice.currency)}</td></tr>)}</tbody></table></section>}
    </>}
    {config && <fieldset className="annual-fieldset" disabled={saving}>
      <section className="panel padded annual-config"><h2>Nastavení výdajů</h2><p>Zvolte režim a druh činnosti. U smíšených příjmů upravte kategorii jednotlivých úhrad níže.</p><div className="field-grid"><label className="field"><span>Výdaje</span><select value={config.expense_mode} onChange={e => update({ ...config, expense_mode: e.target.value as Config['expense_mode'] })}><option value="unselected">Vyberte režim…</option><option value="percentage">Výdajový paušál</option><option value="actual">Skutečné výdaje</option></select></label><label className="field"><span>Výchozí druh činnosti</span><select value={config.default_category || ''} onChange={e => update({ ...config, default_category: e.target.value || null })}><option value="">Vyberte druh…</option>{categories.map(([value, title]) => <option value={value} key={value}>{title}</option>)}</select></label></div></section>
      <section className="panel padded annual-config"><h2>Zařazení bankovních transakcí</h2><p>Ke každému pohybu určete, zda jde o zdanitelný příjem, vrácení příjmu, výdaj nebo položku mimo podnikání. Jeden pohyb můžete rozdělit mezi více kategorií; částky musí dát dohromady hodnotu transakce.</p>{bank.filter(tx => tx.booked.startsWith(`${year}-`)).length ? bank.filter(tx => tx.booked.startsWith(`${year}-`)).map(tx => { const allocations = config.bank_allocations.filter(a => a.bank_id === tx.id); return <div className="annual-bank-row" key={tx.id}><div className="annual-bank-main"><strong>{dateLabel(tx.booked)} · {tx.description || tx.variable_symbol || 'Bankovní pohyb'}</strong><span>{money(tx.amount, tx.currency)}{tx.invoice_id ? ' · propojeno s fakturou' : ''}</span></div>{allocations.map((allocation, index) => <div className="annual-allocation" key={index}><label className="field"><span>Zařazení</span><select value={allocation.kind} onChange={e => setAllocation(tx.id, index, { kind: e.target.value as Allocation['kind'] })}><option value="income">Příjem</option><option value="income_refund">Vrácený příjem</option><option value="expense">Výdaj</option><option value="exclude">Mimo podnikání</option></select></label><label className="field"><span>Částka · {tx.currency}</span><input inputMode="decimal" value={allocation.amount} onChange={e => setAllocation(tx.id, index, { amount: e.target.value })}/></label><label className="field"><span>Druh příjmu</span><select value={allocation.category || ''} onChange={e => setAllocation(tx.id, index, { category: e.target.value })} disabled={allocation.kind === 'exclude'}><option value="">Vyberte…</option>{categories.map(([value, title]) => <option value={value} key={value}>{title}</option>)}</select></label><label className="field"><span>Daňová částka · CZK</span><input inputMode="decimal" value={allocation.taxable_czk || ''} onChange={e => setAllocation(tx.id, index, { taxable_czk: e.target.value })} placeholder={tx.currency === 'CZK' ? 'Dle transakce' : 'Povinné'}/></label><label className="field annual-note"><span>Poznámka</span><input value={allocation.note || ''} onChange={e => setAllocation(tx.id, index, { note: e.target.value })} placeholder="Účel / doklad"/></label>{allocation.kind === 'expense' && <label className="field annual-purchase"><span>Přijatý doklad</span><select value={allocation.purchase_id || ''} onChange={e => setAllocation(tx.id, index, { purchase_id: e.target.value })}><option value="">Bez propojení</option>{purchases.map(p => <option value={p.id} key={p.id}>{p.supplier} · {p.document_number || dateLabel(p.supply_date)}</option>)}</select></label>}<button className="icon-button" type="button" aria-label="Odebrat rozdělení" onClick={() => removeAllocation(tx.id, index)}><X size={16}/></button></div>)}<button className="text-btn" type="button" onClick={() => addAllocation(tx)}><Plus size={14}/>{allocations.length ? 'Přidat další část' : 'Zařadit transakci'}</button></div>; }) : <div className="hint">V tomto roce nejsou evidovány bankovní pohyby.</div>}</section>
      <section className="panel padded annual-config"><div className="panel-head annual-inline-head"><div><h2>Hotovost a další doložené položky</h2><p>Přidejte příjmy a výdaje mimo importované bankovní pohyby. Uveďte datum platby a podklad, aby je bylo možné ověřit.</p></div><button className="secondary" type="button" onClick={() => update({ ...config, manual_entries: [...config.manual_entries, blankEntry(year, config.default_category || '')] })}><Plus size={15}/>Přidat položku</button></div>{config.manual_entries.map(entry => <div className="annual-manual" key={entry.id}><div className="field-grid three"><label className="field"><span>Datum platby</span><input type="date" value={entry.date} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, date: e.target.value } : x) })}/></label><label className="field"><span>Typ</span><select value={entry.kind} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, kind: e.target.value as ManualEntry['kind'] } : x) })}><option value="income">Příjem</option><option value="income_refund">Vrácený příjem</option><option value="expense">Výdaj</option></select></label><label className="field"><span>Druh činnosti</span><select value={entry.category} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, category: e.target.value } : x) })}><option value="">Vyberte…</option>{categories.map(([value, title]) => <option value={value} key={value}>{title}</option>)}</select></label><label className="field"><span>Částka</span><input inputMode="decimal" value={entry.amount} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, amount: e.target.value } : x) })}/></label><label className="field"><span>Měna</span><input value={entry.currency} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, currency: e.target.value.toUpperCase() } : x) })}/></label><label className="field"><span>Daňová částka · CZK</span><input inputMode="decimal" value={entry.taxable_czk} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, taxable_czk: e.target.value } : x) })}/></label><label className="field"><span>Popis</span><input value={entry.description} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, description: e.target.value } : x) })}/></label><label className="field"><span>Podklad / evidence</span><input value={entry.evidence} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, evidence: e.target.value } : x) })}/></label>{entry.currency !== 'CZK' && <label className="field"><span>Kurz a zdroj přepočtu</span><input value={entry.fx_note || ''} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, fx_note: e.target.value } : x) })} placeholder="Kurz, datum a zdroj"/></label>}{entry.kind === 'expense' && <label className="field"><span>Přijatý doklad</span><select value={entry.purchase_id || ''} onChange={e => update({ ...config, manual_entries: config.manual_entries.map(x => x.id === entry.id ? { ...x, purchase_id: e.target.value } : x) })}><option value="">Bez propojení</option>{purchases.map(p => <option value={p.id} key={p.id}>{p.supplier} · {p.document_number || dateLabel(p.supply_date)}</option>)}</select></label>}</div><button className="text-btn" type="button" onClick={() => update({ ...config, manual_entries: config.manual_entries.filter(x => x.id !== entry.id) })}>Odebrat položku</button></div>)}</section>
      {!!purchases.filter(p => p.supply_date.startsWith(`${year}-`)).length && <section className="panel padded annual-config"><h2>Přijaté doklady k ověření</h2><p>Datum dokladu samo o sobě neznamená zaplacený výdaj. Propojte úhradu výše, nebo potvrďte, že se doklad v tomto roce do výdajů nezapočítává či je doložen jinak.</p>{purchases.filter(p => p.supply_date.startsWith(`${year}-`)).map(p => <label className="check annual-purchase-review" key={p.id}><input type="checkbox" checked={!!config.reviewed_purchase_ids?.includes(p.id)} onChange={e => { const ids = config.reviewed_purchase_ids || []; update({ ...config, reviewed_purchase_ids: e.target.checked ? [...ids, p.id] : ids.filter(id => id !== p.id) }); }}/><span>{p.supplier} · {p.document_number || dateLabel(p.supply_date)} · {money(Number(p.base) + Number(p.vat), p.currency)}</span></label>)}</section>}
      <section className="panel padded annual-config"><h2>Ověření nepropojených úhrad</h2><p>Pokud existují hotovostní nebo nepropojené platby, mohou měnit stav pohledávek. Potvrďte jejich samostatnou kontrolu vůči vystaveným fakturám.</p><label className="check"><input type="checkbox" checked={!!config.nonlinked_receivables_reviewed} onChange={e => update({ ...config, nonlinked_receivables_reviewed: e.target.checked })}/>Zkontroloval(a) jsem nepropojené úhrady a pohledávky</label></section>
      <section className="panel padded annual-confirm"><h2>Potvrzení úplnosti roku</h2><p>Potvrďte až po kontrole všech bankovních, hotovostních a jiných příjmů i výdajů. Změna evidence toto potvrzení zneplatní.</p><label className="check"><input type="checkbox" checked={confirmRecords} disabled={year >= new Date().getFullYear()} onChange={e => { setConfirmRecords(e.target.checked); setDirty(true); }}/>{year >= new Date().getFullYear() ? 'Aktuální rok nelze potvrdit jako uzavřený' : 'Ověřil(a) jsem úplnost evidence pro tento rok'}</label>{summary?.records_complete && !dirty && <div className="ok-line"><ShieldCheck size={17}/>Úplnost roku byla potvrzena pro aktuální evidenci.</div>}</section>
      <div className="annual-save"><button className="primary" type="button" disabled={saving || loading || !dirty} onClick={saveConfig}>{saving ? 'Ukládám a přepočítávám…' : 'Uložit a přepočítat rok'}</button>{dirty && <span>Máte neuložené úpravy.</span>}</div>
    </fieldset>}
  </>;
}
