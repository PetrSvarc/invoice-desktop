import { useRef, useState } from 'react';
import { CircleAlert, FileUp, ShieldCheck } from 'lucide-react';
import { dateLabel, money } from './invoice-format';

type PreviewRow = {
  source_id: string;
  number: string;
  issue_date: string;
  customer: string;
  gross: string;
  currency: string;
  status: string;
  reason?: string;
};
type Preview = {
  sha256: string;
  source_format: string;
  total: number;
  importable: number;
  duplicates: number;
  blocked: number;
  can_commit: boolean;
  invoices: PreviewRow[];
  warnings: string[];
};
type CommitSummary = { imported: number; duplicates: number };
type Ask = <T,>(action: string, payload?: Record<string, unknown>) => Promise<T>;

const rowLabel: Record<string, string> = {
  importable: 'Připraveno',
  duplicate: 'Již v archivu',
  blocked: 'Nelze importovat',
};
const decodeXml = (bytes: ArrayBuffer): string => {
  const raw = new Uint8Array(bytes);
  if (raw[0] === 0x50 && raw[1] === 0x4b) throw new Error('Vybraný soubor je ZIP. Nejprve z něj rozbalte XML vystavených faktur.');
  const header = Array.from(raw.slice(0, 256), byte => String.fromCharCode(byte)).join('');
  const declared = header.match(/<\?xml[^?]*encoding\s*=\s*["']([^"']+)["']/i)?.[1]?.toLowerCase() || 'utf-8';
  const encoding = declared === 'utf-8' || declared === 'utf8' ? 'utf-8' : declared === 'windows-1250' || declared === 'cp1250' ? 'windows-1250' : '';
  if (!encoding) throw new Error(`Nepodporované kódování XML: ${declared}. Použijte export UTF-8 nebo Windows-1250.`);
  return new TextDecoder(encoding, { fatal: true }).decode(bytes);
};
const sha256 = async (bytes: ArrayBuffer): Promise<string> => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), byte => byte.toString(16).padStart(2, '0')).join('');

export function IdokladImport({ ask, onImported }: { ask: Ask; onImported: () => Promise<void> }) {
  const [fileName, setFileName] = useState('');
  const [content, setContent] = useState('');
  const [sourceSha256, setSourceSha256] = useState('');
  const [preview, setPreview] = useState<Preview | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [done, setDone] = useState<CommitSummary | null>(null);
  const request = useRef(0);

  const selectFile = async (file?: File) => {
    const current = ++request.current;
    setPreview(null);
    setDone(null);
    setError('');
    setFileName(file?.name || '');
    setContent('');
    setSourceSha256('');
    if (!file) return;
    if (!file.name.toLowerCase().endsWith('.xml')) {
      setError('Vyberte XML soubor vystavených faktur extrahovaný z exportu iDokladu.');
      return;
    }
    if (file.size > 5 * 1024 * 1024) {
      setError('XML soubor je větší než povolených 5 MiB.');
      return;
    }
    setPending(true);
    try {
      const bytes = await file.arrayBuffer();
      const xml = decodeXml(bytes);
      const byteHash = await sha256(bytes);
      const result = await ask<Preview>('idoklad.preview', { content: xml, source_sha256: byteHash });
      if (current !== request.current) return;
      setContent(xml);
      setSourceSha256(byteHash);
      setPreview(result);
    } catch (e) {
      if (current === request.current) setError(String(e));
    } finally {
      if (current === request.current) setPending(false);
    }
  };

  const commit = async () => {
    if (!preview?.can_commit || !content || pending) return;
    setPending(true);
    setError('');
    try {
      const result = await ask<CommitSummary>('idoklad.commit', { content, sha256: preview.sha256, source_sha256: sourceSha256 });
      setDone(result);
      setPreview(null);
      setContent('');
      await onImported();
    } catch (e) {
      setError(String(e));
    } finally {
      setPending(false);
    }
  };

  return <>
    <div className="page-heading"><div><div className="eyebrow">PŘEVOD DAT</div><h1>Import z iDokladu</h1><p>Historické vystavené faktury převedete do místního archivu.</p></div></div>
    <section className="panel padded import-intro">
      <h2>Vyberte export vystavených faktur</h2>
      <p>V iDokladu otevřete <strong>Nastavení → Aplikace → Export → Faktury vydané</strong>. V ZIP exportu rozbalte soubor <code>iDoklad2PohodaExportIssuedInvoice_*.xml</code> a vyberte ho zde. Náhled ukáže nové záznamy, duplicity a důvody případného odmítnutí.</p>
      <label className="upload import-picker"><FileUp size={18}/>Vybrat XML soubor<input aria-label="Vybrat XML soubor z iDokladu" type="file" accept=".xml,text/xml,application/xml" disabled={pending} onChange={e => { void selectFile(e.target.files?.[0]); e.target.value = ''; }}/></label>
      {fileName && <span className="import-file">{fileName}</span>}
      {pending && <div className="hint" role="status">{preview ? 'Importuji vybrané faktury…' : 'Kontroluji soubor a připravuji náhled…'}</div>}
      {error && <div className="issue" role="alert"><CircleAlert size={17}/>{error}</div>}
      {done && <div className="ok-line" role="status"><ShieldCheck size={17}/>Importováno {done.imported} faktur; přeskočeno {done.duplicates} duplicit.</div>}
    </section>
    {preview && <>
      <div className="stats import-stats">
        <div className="stat"><span>V souboru</span><strong>{preview.total}</strong><small>vystavených faktur</small></div>
        <div className="stat accent"><span>Připraveno</span><strong>{preview.importable}</strong><small>nových záznamů</small></div>
        <div className="stat"><span>Duplicity</span><strong>{preview.duplicates}</strong><small>budou přeskočeny</small></div>
        <div className="stat"><span>Chyby</span><strong>{preview.blocked}</strong><small>vyžadují opravu zdroje</small></div>
      </div>
      <section className="panel padded import-review">
        <h2>Kontrola před importem</h2>
        <p>Zdroj: {preview.source_format}. Importované faktury se uloží jako <strong>historický archiv</strong>. Samotné vystavení nepotvrzuje přijetí úhrady a neznámé údaje o DPH se nedopočítají.</p>
        {preview.warnings.map((warning, index) => <div className="issue" key={index}><CircleAlert size={17}/>{warning}</div>)}
        <div className="button-row import-actions"><button className="primary" type="button" disabled={pending || !preview.can_commit || preview.importable === 0} onClick={commit}>Importovat připravené ({preview.importable})</button><button className="secondary" type="button" disabled={pending} onClick={() => { request.current++; setPreview(null); setContent(''); setFileName(''); }}>Zrušit náhled</button></div>
      </section>
      <section className="panel table-panel import-table"><table><thead><tr><th>Číslo</th><th>Odběratel</th><th>Vystaveno</th><th>Částka</th><th>Výsledek</th></tr></thead><tbody>{preview.invoices.map((row, index) => <tr key={`${row.source_id}-${index}`}><td className="strong">{row.number || '—'}</td><td>{row.customer || '—'}</td><td>{row.issue_date ? dateLabel(row.issue_date) : '—'}</td><td>{row.gross && row.currency ? money(row.gross, row.currency) : '—'}</td><td><span className={`badge ${row.status === 'importable' ? 'paid' : row.status === 'duplicate' ? 'draft' : 'overdue'}`}>{rowLabel[row.status] || row.status}</span>{row.reason && <small className="import-reason">{row.reason}</small>}</td></tr>)}</tbody></table></section>
    </>}
  </>;
}
