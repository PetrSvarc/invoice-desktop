# Faktura — private macOS invoicing MVP

Faktura is a local-first invoicing desktop app for a Czech sole trader or small business. The UI is in Czech. It stores operational records in SQLite under the macOS application data directory (`cz.local.faktura/invoices.sqlite`). There is no account or hosted service. The database is not encrypted by the app; use macOS FileVault and protect exported backups.

## Run

Requires current Rust, Node.js and macOS development tools.

```sh
npm install
npm run tauri dev
```

For a distributable app:

```sh
npm run tauri build
```

Checks:

```sh
npm test
npm run build
cd src-tauri && cargo test && cargo check
```

The first launch opens an empty private workspace. You can add an isolated **Ukázkový prostor** from the overview; clearing it deletes the sample records. EPO testing is disabled in demo mode. Public registry lookup also works in demo mode and is read-only.

For QA or a separate local profile, set `INVOICE_DESKTOP_DATA_DIR` to an absolute directory before launching. It moves that run's SQLite database without touching the regular app profile.

## Implemented workflows

- Business profile and dated registrations for **nonpayer**, **identified person**, and **VAT payer**. The regime is selected by supply date. Issued invoices freeze the seller/customer/profile snapshot, and invoice numbers are allocated inside a SQLite transaction by year.
- Customer records; draft invoices with lines, dates, currency, and exact decimal calculations; issued immutable invoice content. Payments are separate bank records. Print an issued invoice and choose **Save as PDF** in the macOS print dialog.
- Issuer payment settings include account holder, bank name, domestic account, IBAN, BIC/SWIFT and default constant symbol. **Doplnit / zkontrolovat IBAN** validates Czech account checksums and fills a missing account representation; mismatched domestic account and IBAN are rejected. This checks format, not account ownership. Invoice variable, constant and specific symbols are saved at issuance; an empty variable symbol defaults to the invoice number's digits.
- Issued CZK invoices with a valid Czech account include a locally generated **QR Platba** beside their payment details. The QR contains the archived IBAN, exact invoice amount, due date and payment symbols. Changing the current business profile cannot redirect payment on an old invoice. Invalid or unsupported inputs suppress the QR and explain why in the preview. This is payment-only QR Platba; QR Platba+F and foreign-currency/SEPA QR are not implemented.
- Monthly/interval recurring templates. Due templates run on startup and every five minutes while the app is open. The engine catches up missed periods and records a unique template/period key, so retries do not issue a second invoice. Month-end anchors are preserved. Failures appear in the exception inbox and job history.
- **Automatic IČO/DIČ lookup** inside the existing customer and business form fields. After a short typing pause, a complete IČO or Czech DIČ fills the name, registered address and available identifiers. The inline status shows the source, verification time, VAT status and reliability. The search icon or Enter also triggers a lookup, including shorter IČO values with omitted leading zeroes. Save the form to keep the changes. Later identifier edits invalidate older requests, and manual edits made while a lookup is pending are preserved. Opening an existing record does not trigger a lookup. Existing invoice snapshots and your dated VAT registration history are not changed. Personal DIČ values are never converted to guessed IČO values.
- CSV transaction import with deduplication, saved variable-symbol plus amount matching and manual links. Matching treats leading zeroes in numeric symbols as equivalent and leaves ambiguous candidates unlinked. Live bank connections have been removed from the app at the owner's request. No bank credentials are needed.
- Incoming purchase records and Czech VAT period preview. For explicitly supported cases, exports a DPHDP3 XML and validates it against the included official XSD with `xmllint`. Tax profile and period-record completeness must be confirmed; new document edits invalidate that confirmation. EPO integration only uses the **test=1** endpoint and requires a user-supplied PKCS#7 signed file whose content exactly matches the generated XML. A successful test is recorded as tested, **never filed**.
- Full JSON workspace backup/restore, including issued snapshots, recurrence keys and tax configuration. Backups are plaintext; store them securely.
- **Import z iDokladu** previews an issued-invoice XML export for POHODA before committing it to a historical archive. Supported records preserve source numbers, customer snapshots, amounts and source XML. Duplicate imports are skipped; conflicting or unsupported records block the batch. Archived documents do not imply payment or a verified VAT treatment. See [import scope and export instructions](docs/idoklad-import.md).
- **Přehled FO** prepares yearly §7 income-tax working papers from received payments and explicitly documented entries. It supports percentage expenses by activity and actual paid expenses, shows year-end receivables and unresolved records, and exports a review CSV. Completing the records is a separate, change-sensitive confirmation; the current year stays provisional. This is supporting evidence for a personal income-tax return, not a complete return or submission. See [annual summary scope](docs/annual-fo.md).

## Boundaries of this MVP

Registry lookup needs internet access and supports Czech IČO and CZ DIČ. Foreign VAT IDs are not supported yet. Provider downtime is reported separately from missing records. Background work runs only while the desktop app is open. Tax XML generation covers limited domestic CZK cases and selected foreign service purchases as defined in the app's readiness checks. Unsupported VAT treatments, foreign-currency returns, missing identity/tax-office fields and incomplete period records block export. KH and SH forms are not generated. Production VAT submission, receipt polling, full iDoklad migration (including payment history, recurring templates and original PDFs), email sending, original purchase attachment archive, and automated backups still require implementation. Annual FO summaries exclude personal deductions, credits and non-business income, and do not generate ČSSZ or health-insurer reports. No production filing is attempted.

Read [integration research](docs/integration-research.md) for verified ARES, VAT-register and EPO contracts, current schemas and integration considerations. These constraints should be resolved with the actual legal entity and filing credentials and accountant before replacing an existing invoicing service.

## CSV import format

UTF-8 CSV with this header, decimal dot amounts and ISO dates:

```csv
id,date,amount,currency,variable_symbol,description
tx-001,2026-10-01,1210.00,CZK,20260001,Invoice payment
```

Importing the same `source + id` twice is idempotent.
