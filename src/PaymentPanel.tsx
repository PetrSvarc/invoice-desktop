import { useEffect, useRef } from 'react';

export type Payment = {
  account_name: string;
  bank_name: string;
  bank_account: string;
  iban: string;
  bic: string;
  variable_symbol: string;
  constant_symbol: string;
  specific_symbol: string;
  method: 'bank_transfer';
  amount: string;
  currency: string;
  due_date: string;
  qr_svg: string | null;
  qr_payload: string | null;
  qr_error: string | null;
};

export type ManualPayment = Pick<Payment, 'account_name' | 'bank_name' | 'bank_account' | 'iban' | 'bic' | 'variable_symbol' | 'constant_symbol' | 'specific_symbol' | 'amount' | 'currency'>;

type Props = {
  payment: Payment | null;
  manual: ManualPayment;
  pending: boolean;
  error: string;
  qrStatus: 'ready' | 'loading' | 'error';
  onQrLoad: () => void;
  onQrError: () => void;
};

const amountLabel = (amount: string, currency: string) => {
  const number = Number(amount);
  return Number.isFinite(number) && /^[A-Z]{3}$/.test(currency)
    ? new Intl.NumberFormat('cs-CZ', { style: 'currency', currency }).format(number)
    : `${amount} ${currency}`;
};

export function PaymentPanel({ payment, manual, pending, error, qrStatus, onQrLoad, onQrError }: Props) {
  const details = payment || manual;
  const qr = payment?.qr_svg && qrStatus !== 'error' ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(payment.qr_svg)}` : null;
  const qrImage = useRef<HTMLImageElement>(null);
  useEffect(() => {
    if (qrStatus !== 'loading' || !qr || !qrImage.current?.complete) return;
    if (qrImage.current.naturalWidth > 0) onQrLoad();
  }, [qr, qrStatus, onQrLoad]);
  const qrMessage = error || (qrStatus === 'error' ? 'QR kód se nepodařilo zobrazit. Použijte platební údaje vedle něj.' : payment?.qr_error || '');
  const rows = [
    ['Majitel účtu', details.account_name],
    ['Název banky', details.bank_name],
    ['Číslo účtu', details.bank_account],
    ['IBAN', details.iban],
    ['BIC / SWIFT', details.bic],
    ['Variabilní symbol', details.variable_symbol],
    ['Konstantní symbol', details.constant_symbol],
    ['Specifický symbol', details.specific_symbol],
  ];

  return <section className="invoice-payment" aria-label="Platební údaje">
    <div className="payment-heading"><div><div className="eyebrow">PLATBA</div><h2>Platební údaje</h2></div><span className="payment-method">Bankovní převod</span></div>
    <div className="payment-content">
      <div className="payment-details">
        {rows.filter(([, value]) => value).map(([label, value]) => <div className="payment-row" key={label}><span>{label}</span><strong>{value}</strong></div>)}
        {!details.bank_account && !details.iban && <div className="payment-row"><span>Účet</span><strong>Neuveden</strong></div>}
      </div>
      <div className="payment-side">
        <div className="payment-amount"><span>Celkem k úhradě</span><strong>{amountLabel(details.amount, details.currency)}</strong></div>
        {qr && <div className="payment-qr"><div className="payment-qr-image"><img ref={qrImage} src={qr} alt="QR Platba pro bankovní převod" onLoad={onQrLoad} onError={onQrError}/></div><strong>QR Platba</strong><small>Naskenujte v bankovní aplikaci</small></div>}
      </div>
    </div>
    {(pending || qrMessage) && <div className="payment-note" role="status">{pending ? 'Načítám platební údaje…' : qrMessage}</div>}
  </section>;
}
