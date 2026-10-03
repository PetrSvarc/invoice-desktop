import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CheckCircle2, LoaderCircle, Search } from 'lucide-react';
import { createSubjectAutofill, type AutofillState, type IdentifierKind, type SubjectIdentity, type SubjectResult } from './subject-autofill';

type Props = {
  value: SubjectIdentity;
  onChange: (patch: Partial<SubjectIdentity>) => void;
  onPendingChange: (pending: boolean) => void;
  children?: ReactNode;
};

export function SubjectFields({ value, onChange, onPendingChange, children }: Props) {
  const [state, setState] = useState<AutofillState>({ phase: 'idle' });
  const latest = useRef({ value, onChange, onPendingChange });
  latest.current = { value, onChange, onPendingChange };
  const statusId = useId();
  const controller = useRef<ReturnType<typeof createSubjectAutofill> | null>(null);
  const update = (fields: Partial<SubjectIdentity>) => {
    latest.current.value = { ...latest.current.value, ...fields };
    latest.current.onChange(fields);
  };
  if (!controller.current) controller.current = createSubjectAutofill({
    read: () => latest.current.value,
    patch: update,
    lookup: (kind, identifier) => invoke<SubjectResult>('dispatch', { action: 'subject.lookup', payload: { kind, identifier } }),
    publish: next => {
      setState(next);
      latest.current.onPendingChange(next.phase === 'waiting' || next.phase === 'loading');
    },
  });
  const lookup = controller.current;
  useEffect(() => { lookup.sync(); }, [value.ico, value.dic, lookup]);
  useEffect(() => () => { lookup.dispose(); latest.current.onPendingChange(false); }, [lookup]);
  const pending = state.phase === 'waiting' || state.phase === 'loading';
  const result = state.result;

  const identifierField = (kind: IdentifierKind, label: string, placeholder: string) => <div className="field identifier-field">
    <label htmlFor={`${statusId}-${kind}`}>{label}</label>
    <div className="identifier-control">
      <input id={`${statusId}-${kind}`} value={value[kind]} placeholder={placeholder} autoComplete="off" spellCheck={false}
        inputMode={kind === 'ico' ? 'numeric' : 'text'} aria-describedby={statusId}
        onChange={e => lookup.edit(kind, e.target.value)}
        onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); lookup.search(kind); } }}/>
      <button type="button" className="identifier-search" title={`Vyhledat podle ${label}`} aria-label={`Vyhledat podle ${label}`}
        disabled={!value[kind].trim() || pending} onClick={() => lookup.search(kind)}>
        {pending && state.kind === kind ? <LoaderCircle size={16} className="lookup-spinner"/> : <Search size={16}/>}
      </button>
    </div>
  </div>;

  return <div className="field-grid subject-fields">
    {identifierField('ico', 'IČO', 'Např. 27074358')}
    {identifierField('dic', 'DIČ', 'Např. CZ27074358')}
    <div id={statusId} className={`identifier-status ${state.phase}`} role="status" aria-live="polite" aria-atomic="true">
      {pending ? <span>Načítám údaje podle {state.kind === 'ico' ? 'IČO' : 'DIČ'}…</span> :
        state.phase === 'error' ? <><span>Údaje se nepodařilo doplnit. {state.message} Můžete je vyplnit ručně.</span><button type="button" className="text-btn" onClick={() => lookup.search(state.kind!)}>Zkusit znovu</button></> :
        result ? <>
          <span className="identifier-success"><CheckCircle2 size={14}/> Doplněno z {result.source}{result.vat_status && ` · ${result.vat_status}`}</span>
          <span className="identifier-detail">Ověřeno {new Date(result.checked_at).toLocaleString('cs-CZ')}{result.reliability && ` · ${result.reliability}`}</span>
          {state.preservedEdits && <span>Vaše ruční úpravy zůstaly zachovány.</span>}
          {result.warnings.map((warning, index) => <span className="identifier-warning" key={index}>{warning}</span>)}
        </> : <span>{state.message || 'Zadejte IČO nebo české DIČ. Název a adresu doplníme automaticky.'}</span>}
    </div>
    <label className="field"><span>Název / jméno</span><input value={value.name} onChange={e => update({ name: e.target.value })}/></label>
    <label className="field"><span>Adresa</span><input value={value.address} onChange={e => update({ address: e.target.value })}/></label>
    {value.country !== undefined && <label className="field"><span>Země</span><input value={value.country} onChange={e => update({ country: e.target.value })}/></label>}
    {children}
  </div>;
}
