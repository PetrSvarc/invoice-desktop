export const money = (s: string | number, currency = 'CZK') => {
  const number = Number(s);
  return Number.isFinite(number) && /^[A-Z]{3}$/.test(currency)
    ? new Intl.NumberFormat('cs-CZ', { style: 'currency', currency }).format(number)
    : `${s} ${currency}`;
};

export const dateLabel = (s: string) => s ? new Date(`${s}T12:00:00`).toLocaleDateString('cs-CZ') : '';

export const treatmentLabel: Record<string, string> = {
  nonpayer: 'Bez DPH · neplátce / IO',
  domestic_standard: 'Tuzemské · 21 %',
  domestic_reduced: 'Tuzemské · 12 %',
  exempt: 'Osvobozené',
  foreign_service_eu: 'Služba z EU · reverse charge',
  foreign_service_non_eu: 'Služba mimo EU · reverse charge',
};
