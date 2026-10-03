export type IdentifierKind = 'ico' | 'dic';
export type SubjectIdentity = {
  name: string;
  ico: string;
  dic: string;
  address: string;
  country?: string;
};
export type SubjectResult = {
  source: string;
  checked_at: string;
  ico: string | null;
  dic: string | null;
  name: string | null;
  address: string | null;
  country: string | null;
  vat_status: string | null;
  reliability?: string | null;
  warnings: string[];
};
export type AutofillState = {
  phase: 'idle' | 'waiting' | 'loading' | 'success' | 'error';
  kind?: IdentifierKind;
  result?: SubjectResult;
  message?: string;
  preservedEdits?: boolean;
};

const compact = (value: string) => value.replace(/\s/g, '').toUpperCase();
const normalized = (kind: IdentifierKind, value: string) => {
  const clean = compact(value);
  if (!clean) return '';
  return kind === 'ico' ? clean.padStart(8, '0') : clean.startsWith('CZ') ? clean : `CZ${clean}`;
};
const complete = (kind: IdentifierKind, value: string) =>
  (kind === 'ico' ? /^\d{8}$/ : /^(?:CZ)?\d{8,10}$/).test(compact(value));

type Options = {
  read: () => SubjectIdentity;
  patch: (patch: Partial<SubjectIdentity>) => void;
  lookup: (kind: IdentifierKind, identifier: string) => Promise<SubjectResult>;
  publish: (state: AutofillState) => void;
};

// Only identifier edits trigger a lookup. Opening a record or filling the other
// identifier from a response must never start a second lookup.
export function createSubjectAutofill(options: Options, delay = 700) {
  let version = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let origin = { ...options.read() };
  let known = { ico: origin.ico, dic: origin.dic };

  const cancel = () => {
    version += 1;
    clearTimeout(timer);
    timer = undefined;
  };
  const patch = (fields: Partial<SubjectIdentity>) => {
    known = { ico: fields.ico ?? options.read().ico, dic: fields.dic ?? options.read().dic };
    options.patch(fields);
  };
  const fetch = async (kind: IdentifierKind, before: SubjectIdentity, token: number) => {
    if (token !== version) return;
    timer = undefined;
    options.publish({ phase: 'loading', kind });
    const isCurrent = () => token === version &&
      options.read().ico === before.ico && options.read().dic === before.dic;
    try {
      const result = await options.lookup(kind, before[kind].trim());
      if (!isCurrent()) return;
      const current = options.read();
      const different = normalized(kind, before[kind]) !== normalized(kind, origin[kind]);
      const fields: Partial<SubjectIdentity> = {};
      let preservedEdits = false;
      for (const key of ['ico', 'dic', 'name', 'address', 'country'] as const) {
        if (key === 'country' && current.country === undefined) continue;
        // A manual edit made during the debounce or network request wins.
        if (current[key] !== before[key]) { preservedEdits = true; continue; }
        fields[key] = result[key]?.trim() || (key === kind ? normalized(kind, before[kind]) :
          different ? '' : current[key]);
      }
      patch(fields);
      origin = { ...options.read(), ...fields };
      options.publish({ phase: 'success', kind, result, preservedEdits });
    } catch (cause) {
      if (isCurrent()) options.publish({ phase: 'error', kind, message: String(cause) });
    }
  };

  return {
    edit(kind: IdentifierKind, value: string) {
      cancel();
      const before = { ...options.read(), [kind]: value };
      patch({ [kind]: value });
      if (!complete(kind, value)) {
        const foreign = kind === 'dic' && /^[A-Z]{2}/.test(compact(value)) && !compact(value).startsWith('CZ');
        options.publish({ phase: 'idle', kind, message: foreign ? 'Automatické doplnění podporuje pouze české DIČ.' : undefined });
        return;
      }
      const token = version;
      options.publish({ phase: 'waiting', kind });
      timer = setTimeout(() => { void fetch(kind, before, token); }, delay);
    },
    search(kind: IdentifierKind) {
      cancel();
      const before = { ...options.read() };
      if (!before[kind].trim()) {
        options.publish({ phase: 'error', kind, message: `Zadejte ${kind === 'ico' ? 'IČO' : 'DIČ'}.` });
        return;
      }
      void fetch(kind, before, version);
    },
    sync() {
      const current = options.read();
      if (current.ico !== known.ico || current.dic !== known.dic) {
        cancel();
        origin = { ...current };
        known = { ico: current.ico, dic: current.dic };
        options.publish({ phase: 'idle' });
      }
    },
    dispose: cancel,
  };
}
