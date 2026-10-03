# Implementation plan and MVP status

## Product scope

Private, macOS-first invoicing for one Czech business. The owner currently needs issued and recurring invoices, is an identified person and expects to become a full VAT payer. The application therefore keeps **effective-dated VAT registrations** rather than a single current switch. It must distinguish invoice issuance, payment reconciliation, tax preparation, testing, and actual filing as separate facts.

## Achieved in this repository

1. **Local desktop foundation.** Tauri 2 hosts a React interface and Rust core backed by SQLite WAL. SQLite is under macOS app data. The same Rust core handles every command; business calculations are `rust_decimal`, never floating point.
2. **Invoice lifecycle.** Customer and business records, drafts, decimal line totals, issue/supply/due dates, VAT treatment checks, transactional yearly invoice numbers, issued seller/customer snapshots, print-to-PDF and separately tracked bank payments. Issuer bank metadata and VS/KS/SS are archived at issuance; valid Czech CZK invoices include local QR Platba. Unsupported tax treatment is rejected. Existing issued invoices are not edited after a profile change.
3. **Recurring issuance.** Persisted templates and template/period uniqueness; local-calendar startup and five-minute checks; catch-up for missed periods; month-end anchors; exception and job history. Errors stop that template until a user fixes it. Idempotency is tested.
4. **Subject lookup and payments.** ARES lookup by IČO and Czech VAT-register lookup by DIČ, integrated directly into customer/business identifier fields with automatic filling after a typing pause. Inline status reports the source and registration status separately from the business's effective-dated tax profile. CSV payments retain deduplication, matching and manual linking. Live bank access is out of scope and its adapter/scheduler/UI have been removed.
5. **Tax preparation.** Incoming purchase entries, period preview, blockers, complete-record confirmation tied to a record fingerprint, limited DPHDP3 XML generation, validation against versioned official XSD, and a test-only EPO adapter that verifies a supplied PKCS#7 file contains the exact XML. No production submission is claimed.
6. **Continuity.** Plain JSON backup/restore includes issued invoices, recurrence run keys and configuration. Demo fixtures are isolated by a demo flag; live connectors are blocked in demo mode.
7. **Historical import and annual FO working papers.** Local iDoklad/POHODA issued-invoice XML import uses preview, duplicate/conflict checks and an atomic archive commit. Source XML and provenance remain in backups. Annual §7 summaries distinguish actual receipts from invoices, support percentage and actual expenses, and export an annotated CSV with unresolved items. See [import details](idoklad-import.md) and [annual summary details](annual-fo.md) for supported cases and remaining boundaries.

## Required next implementation

1. **Tax coverage and production filing:** fully map and test Czech control statement (KH) and EU recapitulative statement (SH), identified-person cases, adjustments, corrections, exemptions, exports and other VAT treatments. Obtain legal/entity/tax-office data and real signing credentials. Add a production EPO adapter with a configured filing policy (review each return or automatically submit eligible periods), receipt/status polling, durable submission IDs, unknown-outcome handling and no blind retry. Arrange accountant review of representative fixtures before enabling an automatic policy. Choosing maximum automation does not remove the need to configure the actual taxpayer and signing channel.
2. **Always-on automation:** decide whether a private hosted companion or local launch agent is acceptable. The desktop scheduler only runs while the app is open. A worker must use the same SQLite transaction boundaries or a safe synchronized store, idempotent work keys and an auditable queue.
3. **Registry enrichment:** extend beyond Czech IČO/DIČ to VIES if foreign VAT IDs are needed; retain source timestamps and distinguish unavailable registries from negative results. No live bank integration is needed.
4. **Output and source documents:** generate archival PDF directly, add foreign-currency/SEPA QR where needed, structured document attachments, immutable source files and retention/backup policy. Domestic CZK QR Platba is implemented. Current print/PDF uses the macOS print dialog; purchase entries store data only.
5. **Migration and delivery:** extend the supported XML archive import with payment history, recurring templates, original PDFs and unsupported invoice types; reconcile against an actual iDoklad export. Add email delivery with an outbox and unknown-send-result reconciliation. Decide on encrypted automated backups and restore drills.
6. **Quality gates:** end-to-end native UI tests, accessibility review, Czech tax golden files for all supported cases, signed production builds and update distribution.

## Release gate

Do not describe this MVP as fully automated tax filing. The release can issue and recur invoices, look up Czech subjects by IČO/DIČ, prepare limited XML and call EPO's test endpoint when a matching signed file exists. Production filing remains blocked until the cases, credentials, submission lifecycle and approval policy are implemented and verified.

## Concrete delivery sequence after this MVP

| Increment | Implementation | Acceptance evidence |
|---|---|---|
| Daily invoicing replacement | Import iDoklad v3 history; configurable next number; archive PDFs and source attachments; credit/correction documents; foreign payment QR if needed; email outbox and delivery receipts | Historical counts, balances and invoice totals reconcile by currency; a selected invoice PDF matches its immutable records; a retry never resends a known successful delivery |
| Reliable automation | Extract Rust domain/adapters into reusable crates; durable jobs with claims, attempts and retry times; macOS Keychain for secrets; single active worker; encrypted scheduled backups | Reboot, sleep/resume and duplicate-worker tests; interrupted-job recovery; restoration on a clean database; registry downtime appears as actionable status |
| Full VAT workflow | Effective-dated rate/rule catalogue; frozen per-line bases/tax amounts and rule version; CZK conversion snapshots; tax-ledger entries distinct from invoice totals; monthly/quarterly periods as applicable; KH/SH/corrections | Golden cases for nonpayer, identified person and payer; period transitions; rounding; foreign services and EU sales; official XSD plus portal validation; reviewed expected totals |
| Unattended filing | Signing integration; EPO or ISDS adapter; filing policy; period snapshot and immutable XML hash; receipt verification, status polling and reconciliation | Credentialed test environment succeeds; timeout after send does not duplicate filing; rejected returns reopen an exception; successful filing retains the official receipt |
| Annual obligations, if in scope | Separate modules for the actual entity's annual income-tax/accounting obligations and, for an applicable OSVČ setup, ČSSZ and health-insurer reports | Confirmed entity and reporting scope; separate official form/transport fixtures; reconciled annual source records |

For maximum automation, use an always-on private worker if invoices and submissions must occur while the Mac is powered off. The desktop should use that worker as the authoritative writer. If local-only is preferred, a launch agent can run while the user is logged in and the Mac is awake; catch-up and missed-deadline alerts remain necessary.

Outstanding configuration facts: currencies; OSVČ versus company; domestic/EU/non-EU sales; source of incoming purchase documents; signing or datová schránka credentials; allowed hosting; annual filing scope. These determine adapters and tax cases, not whether the project can proceed.

The current MVP uses JSON records within SQLite tables for speed of iteration. Before migration of real history, introduce explicit versioned database migrations, unique business/account identities, an append-only audit log and compatibility tests for old issued snapshots. Preserve issued calculations when tax rules change rather than recalculating old PDFs with new rates.

See [integration research](integration-research.md) for primary sources and [verification](verification.md) for the checks actually performed. This is a functional prototype with a scoped tax engine, not a claim of complete Czech accounting coverage.
