# Integration implementation notes

Verified 1 October 2026. These notes support the private macOS MVP. Live bank connections are excluded by the owner's updated scope. Legal entity, currencies, signing credentials, hosting preference, and filing policy are still unconfirmed. Do not interpret fixture data as the user's records.

## iDoklad replacement scope and migration

[iDoklad's recurring invoice workflow](https://www.idoklad.cz/podpora/pravidelne-faktury) generates documents from a template and can email them on schedule. Matching that workflow requires both durable generation and delivery tracking; local generation alone is the first increment.

[Issued invoice exports](https://www.idoklad.cz/podpora/faktury-vydane) include configurable CSV/XLSX lists. Use those to inventory historical numbers and balances, alongside original PDFs and full line data from a suitable export or API. A list export alone must not be treated as a complete invoice archive. Preserve source IDs, historical invoice numbers, original PDFs, payment state and dated VAT registrations. Import into a staging database, reconcile totals by currency and period, and establish the next number before switching issuance to the new app.

New API migration work must use [iDoklad API v3](https://api.idoklad.cz/Help/v3/cs/): [API v2 closes on 5 October 2026](https://www.idoklad.cz/blog/blizi-se-ukonceni-idoklad-api-v2-zkontrolujte-sve-integrace). No private iDoklad account data was read or migrated during this implementation.

## Desktop architecture choice

Use [Tauri 2](https://v2.tauri.app/) with a Rust domain layer and SQLite, plus a React/TypeScript interface. The core must own money calculations, numbering, snapshots, tax decisions and job identities; the interface formats results. This offers a native macOS shell with familiar UI tooling and a portable Rust core. A later background worker can reuse that core. Financial records remain local unless a configured bank, email or filing adapter explicitly needs transmission.

The runtime currently executes jobs while the desktop app is open. Reliable automation while the Mac is asleep or powered off requires an always-on worker; a macOS launch agent alone cannot solve power-off scheduling. A hosted worker should become the single writer for invoice numbering and job claims, with the desktop acting as its client, rather than synchronizing two independently writable SQLite files.

## Public subject lookup — implemented

- [ARES OpenAPI](https://ares.gov.cz/ekonomicke-subjekty-v-be/rest/v3/api-docs) documents `GET /ekonomicke-subjekty/{ico}`. The app reads the official business name, seat, IČO and DIČ when supplied. Registered address text is preferred, with a structured fallback. ARES also provides its record update date and a legal-form code. The general search filter does not offer DIČ search.
- [Czech VAT-register technical documentation](https://adisspr.mfcr.cz/pmd/dokumentace/webove-sluzby-spolehlivost-platcu) defines `getStatusNespolehlivySubjektRozsirenyV2`. It returns subject name, address, type and reliability for a DIČ. Types distinguish VAT payer, identified person, VAT group and unreliable person. The current [WSDL](https://adisrws.mfcr.cz/dpr/axis2/services/rozhraniCRPDPH.rozhraniCRPDPHSOAP?wsdl) advertises the `mojedane.gov.cz` service endpoint used by the app.
- Input normalization keeps leading zeroes, accepts optional `CZ` for Czech DIČ, and rejects foreign identifiers with an explicit message. A Czech DIČ is not automatically converted to IČO. For eight-digit DIČ only, ARES enrichment is accepted when ARES explicitly returns that exact DIČ for the candidate IČO. For personal or group identifiers, fields unavailable from the registry remain unfilled.
- Lookups use TLS verification, fixed official hosts, timeouts, bounded responses, one in-flight request per app and a ten-minute in-memory cache. No bank login or paid registry subscription is required. The input identifier is sent only to the relevant public registry; raw responses are not written to logs or the database.
- The existing IČO and DIČ form fields automatically fetch and fill identity details after a 700 ms typing pause on a complete identifier. An inline search icon or Enter permits an explicit lookup/retry. Source, check time, VAT status and warnings appear below the identifiers without duplicating the form fields. Opening saved records and programmatic autofill do not start requests. New edits cancel queued work and invalidate older responses; manual edits made while waiting are preserved. A different subject clears unavailable old identity fields. VAT history, email and bank details are preserved; issued snapshots are immutable. Changes remain unsaved until the normal form Save action.
- Negative results, maintenance and unknown statuses are distinct. An absent DIČ does not imply nonpayer status, and current registry data does not establish historical registration dates. Current VAT-register status is never used to silently rewrite the user's dated tax profile.

Live bank synchronization was removed from the app and implementation plan. CSV payment imports and matching remain available.

## Issuer account details and payment QR

The [QR Platba format specification](https://qr-platba.cz/pro-vyvojare/specifikace-formatu/) and [Czech Banking Association standard, version 1.2](https://qr-platba.cz/wp-content/uploads/1645-standard-qr-v1-2-cerven-2021.pdf) define the payment payload. A one-time credit transfer uses `SPD*1.0`, `ACC` for IBAN, `AM` for the decimal amount, `CC` for currency, `DT` for the due date, and `X-VS`, `X-KS`, `X-SS` for payment symbols. The standard recommends error correction M. The payload version remains 1.0 even though the standard document is version 1.2.

The implementation targets Czech CZK transfers. It generates SVG locally with a white quiet zone, without a remote QR service. It is labelled **QR Platba**; the combined payment/invoice format **QR Platba+F** shown in the inspiration is a separate future feature. Other currencies continue to show manual payment details and an explanation when QR generation is unsupported.

Account normalization follows the [ČNB IBAN description](https://www.cnb.cz/cs/platebni-styk/iban/iban-mezinarodni-cislo-uctu/) and [CERTIS account checks](https://www.cnb.cz/export/sites/cnb/cs/platebni-styk/.galleries/certis/download/pravidla_CERTIS_priloha_01.pdf): Czech account components use modulo 11, and IBAN uses its international checksum. The official example `19-2000145399/0800` corresponds to `CZ6508000000192000145399`. A domestic account and IBAN supplied together must identify the same account. These checks establish structural consistency, not account existence or ownership. BIC/SWIFT is displayed separately; the domestic QR needs only the validated IBAN.

Application decisions: freeze issuer account details and payment symbols at issuance; use the exact archived invoice total; retain leading zeroes in symbols; reject unsupported amounts or invalid payment inputs instead of generating a misleading QR. Saving a newer business profile must not redirect payments on an older invoice. Account holder and bank name are user-entered metadata, with the issuer name as the account-holder fallback. The screenshot is visual inspiration and never a source of defaults for the user's bank account.

## EPO: actual validation and filing lifecycle

Source: [official third-party interface specification, v1.11](https://adisspr.mfcr.cz/dpr/adis/idpr_pub/epo2_info/PodatelnaEPO.pdf).

- Submission: `https://mojedane.gov.cz/dpr/epo_podani`.
- Status: `https://mojedane.gov.cz/dpr/epo_stav`.
- Direct submissions require a recognized electronic signature and the specified PKCS#7 encoding.
- `test=1` validates without filing. A `TEST_REZIM` informational response explicitly means no filing occurred. Other returned errors must still be interpreted.
- Large submissions have different validation/receipt behavior; restrict the MVP to the supported small-document flow.

Implementation decisions: distinguish locally generated, XSD-valid, EPO-test-validated, awaiting authorization, submitted, received, processing-failed, and outcome-unknown states. Never promote XML export to submitted status. An uncertain transport outcome requires reconciliation before retry. Archive the exact document, hash, rule version, validation responses, external identifiers, and receipts. Production transmission is disabled until the user configures an authorized channel and policy. Credential setup is distinct from choosing an automatic filing policy.

Alternative: [ISDS's application interface](https://datovka.gov.cz/info/cs/74.html) is available to authorized mailbox users and has a test environment. Authentication and the receipt workflow must be proved before implementing a production adapter.

## Official filing schemas

Downloaded XSDs are in `schemas/`; `schemas/manifest.json` records source URLs and SHA-256 hashes. They have no external schema imports. The source documents are:

- [DPHDP3 — VAT return](https://adisspr.mfcr.cz/dpr/adis/idpr_pub/epo2_info/popis_struktury_detail.faces?zkratka=DPHDP3)
- [DPHKH1 — control statement](https://adisspr.mfcr.cz/dpr/adis/idpr_pub/epo2_info/popis_struktury_detail.faces?zkratka=DPHKH1)
- [DPHSHV — EU recapitulative statement](https://adisspr.mfcr.cz/dpr/adis/idpr_pub/epo2_info/popis_struktury_detail.faces?zkratka=DPHSHV)

The current DPHDP3 page reports 03.01.04 dated 11 August 2026; search snippets can be stale. Use the pinned schemas for structural tests and check published changes before a release.

Run `python3 docs/validate_tax_xml.py path/to/document.xml`. Validation reports structure only; it does not establish tax correctness or submit anything. In particular, required tax-office identifiers, taxpayer identification, legal form, period, and document type cannot be replaced with made-up defaults for live filings.

## Acceptance cases for the implementation

These are proposed engineering checks, not additional claims from the API documentation.

1. Reimporting the same bank statement produces no additional payments.
2. A partial payment changes outstanding balance but never alters the issued invoice total.
3. Currency mismatch and duplicate variable-symbol candidates remain unresolved.
4. Repeated recurrence execution yields one document for each template and period.
5. A registration transition preserves past documents and re-evaluates new periods.
6. An identified-person profile cannot generate a control statement as an applicable filing; see [Financial Administration applicability guidance](https://financnisprava.gov.cz/cs/dane/dane/dan-z-pridane-hodnoty/kontrolni-hlaseni-DPH/kdo-podava-kontrolni-hlaseni).
7. Unsupported tax categories and missing purchase evidence block filing readiness rather than silently contributing zero.
8. XML escaping preserves arbitrary customer and supplier names.
9. Tax return amounts use the form's rounding rules independently of invoice-level precision.
10. Missing credentials, connection failures, and test responses never create a production-success event.
11. An unchanged rerun uses the same internal job identity; changed data requires a new version.
12. Restore into a new database preserves issued documents, payment allocations, and job history.
