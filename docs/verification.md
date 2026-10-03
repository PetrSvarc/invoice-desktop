# Verification — 1–3 October 2026

## iDoklad import and annual FO summaries — 3 October

The importer is tested with schema-shaped synthetic XML, not a real customer export. Its isolated SQLite tests cover duplicate re-imports, changed records and number conflicts, atomic rejection, forbidden XML constructs, namespace handling, preserved source data, and backup restoration. The annual tests cover cross-year and partial receipts, VAT allocation, split activity categories, percentage caps, unresolved FX and expenses, archived-record evidence, CSV output and confirmation invalidation.

CUA browser inspection used `qa/preview.html`, an isolated fixture rendering the actual new React components with mocked responses. No private records were imported or modified. At 1280 × 720, the summary and import layouts have no visible overlaps; classification selects and adjacent amount inputs are both 38 px high. Changing expense mode marks figures as stale and disables CSV until saved. Adding a bank classification and manual entry exposes the expected inputs. A preview with an unsupported row disables import. These checks verify frontend behavior and layout; calculation and database behavior are covered by the Rust tests.

Native macOS inspection also confirmed both new navigation entries and actual backend summary responses. The empty 2025 workspace showed missing expense-method and completeness checks. Selecting 2026 added the provisional-year warning and disabled year-end confirmation. The native save dialog successfully exported `.qa-data/fo-native-empty-2026.csv`; file inspection confirmed `Úplné,false`, `Průběžné,true`, and the matching unresolved reasons. No business configuration, invoices, payments or tax confirmations were saved during this check.

The import scope is documented in [idoklad-import.md](idoklad-import.md), including formats that are rejected rather than guessed. Annual [FO working papers](annual-fo.md) are not a full tax return or filing operation. Receivable balances explicitly describe their linked-bank-payment basis; cash and unlinked settlements need reconciliation.

## Build and automated tests

- `npm run build`: TypeScript and Vite production build pass.
- `npm test`: 14 frontend tests pass (Node with TypeScript stripping support). Nine cover autofill debounce, responses arriving out of order, manual edits during lookup, missing identifiers, DIČ-only results, closing forms, error/retry, external refresh and avoiding requests on opening records or entering foreign DIČ. Three cover payment-panel data consistency, QR refusal and fallback to archived payment details. Two cover local-time greeting boundaries and complete saved names, including academic titles, company punctuation and empty names.
- `cargo test --locked --offline --manifest-path src-tauri/Cargo.toml`: 44 tests pass. Two tests are ignored by default: the explicitly opt-in live registry test and synthetic payment-fixture exporter. Annual regressions include cent-level VAT rounding carried across years and later-year confirmation invalidation after earlier-year classification changes.
- `npm run tauri build -- --debug --bundles app`: builds `src-tauri/target/debug/bundle/macos/Faktura.app`.
- Rust 1.87 compatibility is recorded in the package manifest and lockfile. Tauri JS API 2.11 and dialog plugin 2.7 match the resolved Rust minor versions.

Tests cover decimal totals, dated registration selection, month-end recurrence, repeated generation, immutable seller snapshots, bank CSV deduplication and matching, atomic rejection of malformed CSV, restoration of recurrence keys, IČO/DIČ normalization, registry identity matching, missing DIČ, address construction, identified-person versus payer status, registry outage/fault/not-found handling, identified-person self-assessment without input deduction, supported payer deductions, form rounding, XML escaping and official XSD validation, source-record changes invalidating period confirmation, unsupported tax cases, EPO test-result interpretation, and exact signed-XML matching using a disposable local certificate.

## Native macOS smoke test

The subsequent dashboard greeting fix uses the full trimmed saved business name. Local Mac time selects morning (05:00–09:59), daytime (10:00–17:59) or evening otherwise. The clock refreshes at minute boundaries and on returning to focus/visibility. After rebuilding, the old app process was restarted and native accessibility inspection confirmed the full saved name in the greeting.

The app was launched using its native bundle with synthetic demo records. No private iDoklad history was imported.

1. Empty first-run screen and demo seeding work.
2. The demo workspace persists across restarting the app.
3. Issued invoice preview displays the seller/customer snapshot, dates, VAT rate, base, tax, total and variable symbol.
4. Native macOS print dialog opens with one page. Save as PDF produced `.qa-data/sample-invoice.pdf`.
5. Importing `.qa-data/sample-bank.csv` produces one transaction. Automatic matching links it to `2026-0001`, and the invoice list shows `Zaplaceno`.
6. The tax screen initially blocks export for missing taxpayer data and period confirmation. Saving the synthetic company profile enables XML export. The exported `.qa-data/DPHDP3-2026-10.xml` passes `python3 docs/validate_tax_xml.py` against the pinned official schema. Its domestic base is CZK 12,500, VAT CZK 2,625.
7. EPO network testing remains disabled in demo mode. Live bank connections have been removed.

The native UI run used the app's new default macOS data directory and left it in clearly labelled demo mode. `.qa-data` contains only synthetic test exports. The configurable data-directory override exists for development; direct GUI launch from the execution sandbox aborted, so that launch route was not used for the successful native smoke test.

## Public registry integration check

`cargo test --locked --manifest-path src-tauri/Cargo.toml live_public_registry_lookup -- --ignored --nocapture` passes against the real ARES and Czech VAT-register services. It queried the public company IČO `27074358` and DIČ `CZ27074358`, verified the same identity across both paths and the returned VAT-payer status. This sends read-only registry queries and does not access bank accounts.

The earlier native macOS lookup implementation was checked with these public identifiers. Both lookup modes displayed Asseco Central Europe, a.s., its address, IČO, DIČ, VAT-payer status, reliability and verification time. Applying the preview filled the correct identity and address. Entering a foreign DIČ cleared the old preview and displayed the unsupported-identifier explanation. The unsaved test customer was cancelled.

The subsequent integrated-autofill revision removes that preview/apply section and puts lookup in the existing IČO and DIČ fields. Its TypeScript/Vite and native bundle builds pass. Native visual inspection confirms the compact business form, inline search controls, and consistent select/input sizing. Automatic filling and race handling are covered by the frontend tests; a new end-to-end automatic lookup was not completed during native inspection because the user was interacting with the app concurrently. No test subject was saved during this inspection.

## Issuer accounts and QR Platba

The Rust payment tests cover the ČNB account/IBAN vector, prefix and account checksums, inconsistent account representations, invalid BIC/check digits/bank code, exact amounts and symbols, leading zeroes, invalid or unsupported QR cases, dated payment data, legacy JSON defaults, issuance snapshots and matching by saved VS. The QR uses the archived invoice amount and seller data.

The ignored `export_synthetic_payment_fixture` test was run with `PAYMENT_FIXTURE_PATH` set to this project's `.qa-data/payment-sample.json`. It created a synthetic invoice, its breakdown and actual payment API response, plus `.qa-data/payment-sample.svg`. No user database was opened or modified for these checks.

An independent decoder verified the actual SVG image. The bundled `sharp` renderer converted it to both 256×256 and 1067×1067 PNGs. `scripts/decode-qr.swift` uses Apple Vision and decoded exactly one QR at each size, matching `payment.qr_payload` byte-for-byte. Its fields are ACC, AM, CC, X-VS, X-KS, X-SS and DT. Blank and missing images correctly fail. The Vision runtime required approved execution outside the sandbox in this environment.

`node scripts/preview-payment.mjs` rendered the same `InvoiceSheet` and `PaymentPanel` used by the app with the backend fixture and application CSS. CUA visual inspection under print-media emulation at the usable A4 size (703×1032 CSS pixels with 12 mm margins) found the one-line invoice occupied 896.67 pixels in height, with no clipping or overlaps. The QR image is 41 mm and the payment block is kept together. Direct PDF export was unavailable in the in-app browser, so this is print-CSS and image verification, not a new native PDF-export test or a physical banking-app scan. The native bundle was rebuilt successfully; the user's running app was not restarted.

## Unverified external operations

No real Fio token was used, no live EPO validation request was sent, no production return was submitted, and no emails were sent. XSD validation proves structure, not completeness or correctness of a real tax return. The bundle is a local debug build, not a signed/notarized distribution release. KH, SH, production receipts, unattended hosting, migration beyond the documented XML subset and additional tax cases remain planned work.
