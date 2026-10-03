import { PaymentPanel, type ManualPayment, type Payment } from './PaymentPanel';
import { dateLabel, money, treatmentLabel } from './invoice-format';

export type Breakdown = { treatment: string; rate: string; base: string; vat: string };
export type PrintInvoice = {
  id: string;
  number: string | null;
  issue_date: string;
  supply_date: string;
  due_date: string;
  currency: string;
  lines: { description: string; quantity: string; unit_price: string; treatment: string }[];
  totals: { base: string; vat: string; gross: string };
  regime: string | null;
  seller: { name: string; address: string; ico: string; dic: string; bank_account: string; iban: string; account_name?: string; bank_name?: string; bic?: string; constant_symbol?: string } | null;
  customer: { name: string; address: string; ico: string; dic: string } | null;
  variable_symbol?: string;
  constant_symbol?: string;
  specific_symbol?: string;
};

type Props = {
  invoice: PrintInvoice;
  breakdown: Breakdown[];
  payment: Payment | null;
  paymentPending: boolean;
  paymentError: string;
  qrStatus: 'ready' | 'loading' | 'error';
  onQrLoad: () => void;
  onQrError: () => void;
};

export const manualPayment = (invoice: PrintInvoice): ManualPayment => ({
  account_name: invoice.seller?.account_name || invoice.seller?.name || '',
  bank_name: invoice.seller?.bank_name || '',
  bank_account: invoice.seller?.bank_account || '',
  iban: invoice.seller?.iban || '',
  bic: invoice.seller?.bic || '',
  variable_symbol: invoice.variable_symbol || invoice.number?.replace(/\D/g, '') || '',
  constant_symbol: invoice.constant_symbol || invoice.seller?.constant_symbol || '',
  specific_symbol: invoice.specific_symbol || '',
  amount: invoice.totals.gross,
  currency: invoice.currency,
});

export function InvoiceSheet({ invoice, breakdown, payment, paymentPending, paymentError, qrStatus, onQrLoad, onQrError }: Props) {
  return <article className="invoice-sheet">
    <div className="invoice-sheet-top">
      <div><div className="invoice-logo">faktura.</div><p>{invoice.seller?.name}<br/>{invoice.seller?.address}<br/>IČO {invoice.seller?.ico} {invoice.seller?.dic && `· DIČ ${invoice.seller.dic}`}</p></div>
      <div className="right"><div className="eyebrow">FAKTURA</div><h1>{invoice.number}</h1><p>Vystaveno {dateLabel(invoice.issue_date)}<br/>DUZP {dateLabel(invoice.supply_date)}<br/>Splatnost {dateLabel(invoice.due_date)}</p></div>
    </div>
    <div className="invoice-sheet-customer"><span>ODBĚRATEL</span><strong>{invoice.customer?.name}</strong><div>{invoice.customer?.address}</div><div>IČO {invoice.customer?.ico} {invoice.customer?.dic && `· DIČ ${invoice.customer.dic}`}</div></div>
    <table><thead><tr><th>Popis</th><th>Množství</th><th>Jednotková cena</th><th>Režim</th></tr></thead><tbody>{invoice.lines.map((line,index)=><tr key={index}><td>{line.description}</td><td>{line.quantity}</td><td>{money(line.unit_price,invoice.currency)}</td><td>{treatmentLabel[line.treatment]||line.treatment}</td></tr>)}</tbody></table>
    <div className="invoice-total">
      {breakdown.map(group=><div key={group.treatment}><span>Základ {group.rate} / DPH</span><strong>{money(group.base,invoice.currency)} / {money(group.vat,invoice.currency)}</strong></div>)}
      <div><span>Základ</span><strong>{money(invoice.totals.base,invoice.currency)}</strong></div>
      <div><span>DPH</span><strong>{money(invoice.totals.vat,invoice.currency)}</strong></div>
      <div className="grand"><span>Celkem k úhradě</span><strong>{money(invoice.totals.gross,invoice.currency)}</strong></div>
    </div>
    {(invoice.regime === 'nonpayer' || invoice.regime === 'identified') && <div className="nonpayer-statement">Nejsem plátce DPH.</div>}
    <PaymentPanel payment={payment} manual={manualPayment(invoice)} pending={paymentPending} error={paymentError} qrStatus={qrStatus} onQrLoad={onQrLoad} onQrError={onQrError}/>
    <div className="sheet-footer">Vystaveno v aplikaci faktura. · archivní snímek vystaveného dokladu</div>
  </article>;
}
