//! Versioned VAT preparation and an explicitly non-filing EPO test adapter.
use crate::core::{self, Business, Result, Snapshot};
use chrono::{Datelike, NaiveDate, Utc};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::{collections::BTreeMap, path::PathBuf, process::Command, str::FromStr};

const SCHEMA: &str = include_str!("../../docs/schemas/dphdp3.xsd");
const RULE_VERSION: &str = "cz-domestic-and-service-input-v1";

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    tax_office: String,
    tax_office_branch: String,
    legal_form: String,
    first_name: String,
    last_name: String,
    company_name: String,
    street: String,
    city: String,
    postal_code: String,
    nace: String,
    records_complete: bool,
    confirmed_period: String,
    confirmed_fingerprint: String,
    signed_test_file: String,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn dec(s: &str) -> Result<Decimal> {
    Decimal::from_str(s).map_err(|_| "Invalid decimal in tax records".into())
}
fn money(n: Decimal) -> String {
    format!(
        "{:.2}",
        n.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
    )
}
fn crowns(n: Decimal) -> Decimal {
    n.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
}
fn text<'a>(v: &'a Value, name: &str) -> &'a str {
    v.get(name).and_then(Value::as_str).unwrap_or("")
}
fn period_dates(period: &str) -> Result<(NaiveDate, NaiveDate)> {
    if period.len() != 7 {
        return Err("Use a tax period in YYYY-MM format".into());
    }
    let first = NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d")
        .map_err(|_| "Invalid tax period")?;
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    }
    .ok_or("Invalid tax period")?;
    Ok((first, next.pred_opt().ok_or("Invalid tax period")?))
}
fn in_period(date: &str, period: &str) -> bool {
    date.len() == 10 && date.starts_with(&format!("{period}-"))
}
fn fingerprint(s: &Snapshot, period: &str) -> Result<String> {
    let invoices: Vec<_> = s
        .invoices
        .iter()
        .filter(|i| in_period(&i.supply_date, period))
        .collect();
    let purchases: Vec<_> = s
        .purchases
        .iter()
        .filter(|p| in_period(&p.supply_date, period))
        .collect();
    let bytes = serde_json::to_vec(
        &json!({"period":period,"business":s.business,"invoices":invoices,"purchases":purchases}),
    )
    .map_err(error)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
async fn config(pool: &SqlitePool) -> Result<Config> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key='tax_config'")
        .fetch_optional(pool)
        .await
        .map_err(error)?;
    raw.map(|s| serde_json::from_str(&s).map_err(error))
        .unwrap_or_else(|| Ok(Config::default()))
}
pub async fn tax_config(pool: &SqlitePool) -> Result<Value> {
    serde_json::to_value(config(pool).await?).map_err(error)
}
pub async fn save_tax_config(pool: &SqlitePool, payload: Value) -> Result<Value> {
    let mut cfg: Config = serde_json::from_value(payload.clone()).map_err(error)?;
    if cfg.records_complete {
        let period = text(&payload, "period");
        period_dates(period)?;
        cfg.confirmed_period = period.to_owned();
        cfg.confirmed_fingerprint = fingerprint(&core::snapshot(pool).await?, period)?;
    } else {
        cfg.confirmed_period.clear();
        cfg.confirmed_fingerprint.clear();
    }
    sqlx::query("INSERT INTO meta(key,value) VALUES('tax_config',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(serde_json::to_string(&cfg).map_err(error)?).execute(pool).await.map_err(error)?;
    serde_json::to_value(cfg).map_err(error)
}

#[derive(Default)]
struct Summary {
    values: BTreeMap<String, Decimal>,
    blockers: Vec<String>,
    warnings: Vec<String>,
    regime: String,
    dic: String,
    invoice_count: usize,
    purchase_count: usize,
    output: Decimal,
    input: Decimal,
}
impl Summary {
    fn add(&mut self, field: &str, value: Decimal) {
        *self.values.entry(field.into()).or_default() += value;
    }
    fn sum(&self, fields: &[&str]) -> Decimal {
        fields
            .iter()
            .map(|f| crowns(*self.values.get(*f).unwrap_or(&Decimal::ZERO)))
            .sum()
    }
}
fn tax_dic(b: &Business, on: &str) -> String {
    let r = b
        .registrations
        .iter()
        .filter(|r| r.from.as_str() <= on)
        .max_by_key(|r| &r.from);
    r.map(|r| r.dic.as_str())
        .filter(|d| !d.is_empty())
        .unwrap_or(&b.dic)
        .trim()
        .trim_start_matches("CZ")
        .to_owned()
}
fn prepare(s: &Snapshot, cfg: &Config, period: &str) -> Result<Summary> {
    let (start, end) = period_dates(period)?;
    let mut out = Summary {
        regime: core::regime_at(&s.business, &start.to_string())?,
        dic: tax_dic(&s.business, &end.to_string()),
        ..Summary::default()
    };
    if core::regime_at(&s.business, &end.to_string())? != out.regime
        || s.business
            .registrations
            .iter()
            .any(|r| r.from > start.to_string() && r.from <= end.to_string())
    {
        out.blockers.push("Registration changes within this month require a reviewed partial-period filing; not supported by this exporter yet.".into());
    }
    if out.regime == "nonpayer" {
        out.blockers.push("A standard VAT return is not applicable to this non-payer profile. Special non-payer cases require separate handling.".into());
    }
    if cfg.tax_office.is_empty()
        || !cfg.tax_office.bytes().all(|c| c.is_ascii_digit())
        || cfg.tax_office.len() > 3
    {
        out.blockers
            .push("Configure a valid tax-office code.".into());
    }
    if out.dic.is_empty() || out.dic.len() > 10 || !out.dic.bytes().all(|c| c.is_ascii_digit()) {
        out.blockers
            .push("Configure the taxpayer DIČ for the selected registration period.".into());
    }
    if !["F", "P"].contains(&cfg.legal_form.as_str()) {
        out.blockers
            .push("Select individual (F) or company (P) taxpayer type.".into());
    }
    if cfg.legal_form == "F"
        && (cfg.first_name.trim().is_empty() || cfg.last_name.trim().is_empty())
    {
        out.blockers
            .push("Enter the individual's first and last name.".into());
    }
    if cfg.legal_form == "P" && cfg.company_name.trim().is_empty() {
        out.blockers.push("Enter the company name.".into());
    }
    if cfg.street.trim().is_empty()
        || cfg.city.trim().is_empty()
        || cfg.postal_code.trim().is_empty()
    {
        out.blockers
            .push("Complete the structured taxpayer address.".into());
    }
    if !cfg.records_complete
        || cfg.confirmed_period != period
        || cfg.confirmed_fingerprint != fingerprint(s, period)?
    {
        out.blockers.push("Confirm that this period's source records are complete. Changes to records invalidate an earlier confirmation.".into());
    }
    for inv in s
        .invoices
        .iter()
        .filter(|i| in_period(&i.supply_date, period))
    {
        if inv.status == "archived" {
            out.blockers.push(format!(
                "Imported historical invoice {} has no verified VAT regime or tax details; review its original document before preparing this period's return.",
                inv.number.as_deref().unwrap_or(&inv.id)
            ));
            continue;
        }
        if inv.status != "issued" {
            out.blockers.push(format!(
                "Draft invoice {} falls in this period; resolve it before preparing the return.",
                inv.id
            ));
            continue;
        }
        out.invoice_count += 1;
        if inv.currency != "CZK" {
            out.blockers.push(format!("Invoice {} needs an explicit CZK tax conversion; foreign-currency filing is not implemented.",inv.number.as_deref().unwrap_or(&inv.id)));
            continue;
        }
        if inv.regime.as_deref() != Some(out.regime.as_str()) {
            out.blockers.push(
                "An invoice's issued registration does not match this period's profile.".into(),
            );
            continue;
        }
        for line in &inv.lines {
            let total = match core::calculate(std::slice::from_ref(line), &out.regime) {
                Ok(t) => t,
                Err(e) => {
                    out.blockers.push(e);
                    continue;
                }
            };
            match line.treatment.as_str() {
                "domestic_standard" => { out.add("obrat23",dec(&total.base)?); out.add("dan23",dec(&total.vat)?); },
                "domestic_reduced" => { out.add("obrat5",dec(&total.base)?); out.add("dan5",dec(&total.vat)?); },
                "nonpayer" if out.regime == "identified" => {},
                other => out.blockers.push(format!("Invoice treatment '{other}' requires a tax mapping not supported by this exporter.")),
            }
        }
    }
    for p in s
        .purchases
        .iter()
        .filter(|p| in_period(&p.supply_date, period))
    {
        out.purchase_count += 1;
        let val = serde_json::to_value(p).map_err(error)?;
        if p.currency != "CZK" {
            out.blockers.push(format!(
                "Purchase {} needs an explicit CZK conversion.",
                p.document_number
            ));
            continue;
        }
        let base = dec(&p.base)?;
        let vat = dec(&p.vat)?;
        if base < Decimal::ZERO || vat < Decimal::ZERO {
            out.blockers.push("Purchase corrections require a reviewed correction mapping; negative records are not supported yet.".into());
            continue;
        }
        if p.supplier.trim().is_empty() || p.document_number.trim().is_empty() {
            out.blockers
                .push("Purchase records require supplier and source-document number.".into());
        }
        let rate = match p.treatment.as_str() {
            "domestic_standard" => "21",
            "domestic_reduced" => "12",
            _ => text(&val, "vat_rate"),
        };
        let suffix = match rate {
            "21" => "23",
            "12" => "5",
            _ => {
                out.blockers.push(format!(
                    "Purchase {} needs an explicit supported VAT rate (21 or 12).",
                    p.document_number
                ));
                continue;
            }
        };
        match p.treatment.as_str() {
            "domestic" | "domestic_purchase" | "domestic_vat" | "domestic_standard"
            | "domestic_reduced" => {
                if out.regime == "payer" {
                    out.add(&format!("pln{suffix}"), base);
                    out.add(&format!("odp_tuz{suffix}_nar"), vat);
                }
            }
            "foreign_service_eu" | "foreign_service_non_eu" => {
                let assessed = (base * dec(rate)? / Decimal::ONE_HUNDRED)
                    .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
                if vat != Decimal::ZERO {
                    out.blockers.push(format!("Foreign-service purchase {} includes supplier VAT; review its tax treatment before self-assessment.",p.document_number));
                    continue;
                }
                let territory = if p.treatment == "foreign_service_eu" {
                    "e"
                } else {
                    "z"
                };
                out.add(&format!("p_sl{suffix}_{territory}"), base);
                out.add(&format!("dan_psl{suffix}_{territory}"), assessed);
                if out.regime == "payer" {
                    out.add(&format!("nar_zdp{suffix}"), base);
                    out.add(&format!("od_zdp{suffix}"), assessed);
                }
            }
            other => out.blockers.push(format!(
                "Purchase treatment '{other}' is not supported by this filing exporter."
            )),
        }
    }
    out.output = out.sum(&[
        "dan23",
        "dan5",
        "dan_psl23_e",
        "dan_psl5_e",
        "dan_psl23_z",
        "dan_psl5_z",
    ]);
    out.input = out.sum(&["odp_tuz23_nar", "odp_tuz5_nar", "od_zdp23", "od_zdp5"]);
    if out.regime == "payer" {
        out.add("odp_sum_nar", out.input);
    }
    let has_assessed_service = out
        .values
        .iter()
        .any(|(field, base)| field.starts_with("p_sl") && *base > Decimal::ZERO);
    if out.regime == "identified" && !has_assessed_service {
        out.blockers.push("No supported VAT-payable transaction is recorded for this identified-person period; a regular zero VAT return is not generated automatically.".into());
    }
    out.warnings.push("Monthly regular DPHDP3 only. Domestic input entries mean full deductible business use with the source document held in this period; mixed use, assets, advances, corrections and other special cases need review.".into());
    out.warnings.push("Control statements and EU recapitulative statements require additional mappings and are not generated by this MVP exporter.".into());
    out.blockers.sort();
    out.blockers.dedup();
    Ok(out)
}
pub async fn tax_preview(pool: &SqlitePool, period: &str) -> Result<Value> {
    let s = core::snapshot(pool).await?;
    let cfg = config(pool).await?;
    let p = prepare(&s, &cfg, period)?;
    Ok(
        json!({"period":period,"regime":p.regime,"output_vat":money(p.output),"input_vat":money(p.input),"payable":money(p.output-p.input),"invoice_count":p.invoice_count,"purchase_count":p.purchase_count,"blockers":p.blockers,"warnings":p.warnings,"can_export":p.blockers.is_empty(),"rule_version":RULE_VERSION,"forms":[{"code":"DPHDP3","status":"supported_cases_only"},{"code":"DPHKH1","status":if p.regime=="identified"{"not_applicable"}else{"not_implemented"}},{"code":"DPHSHV","status":"not_implemented"}]}),
    )
}
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn attr(xml: &mut String, key: &str, value: &str) {
    if !value.is_empty() {
        xml.push_str(&format!(" {key}=\"{}\"", escaped(value)));
    }
}
fn section(xml: &mut String, tag: &str, fields: &[&str], p: &Summary) {
    let present: Vec<_> = fields
        .iter()
        .filter_map(|k| p.values.get(*k).map(|v| (*k, crowns(*v))))
        .collect();
    if present.is_empty() {
        return;
    }
    xml.push_str(&format!("<{tag}"));
    for (k, v) in present {
        attr(xml, k, &v.to_string());
    }
    xml.push_str("/>\n");
}
fn render(cfg: &Config, p: &Summary, period: &str) -> Result<String> {
    let (start, _) = period_dates(period)?;
    let mut xml=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Pisemnost nazevSW=\"Invoice Desktop\" verzeSW=\"0.1.0\">\n<DPHDP3 verzePis=\"03.01.04\">\n<VetaD dokument=\"DP3\" k_uladis=\"DPH\" dapdph_forma=\"B\" rok=\"{}\" mesic=\"{}\" typ_platce=\"{}\"",start.year(),start.month(),if p.regime=="identified"{"I"}else{"P"});
    attr(&mut xml, "c_okec", &cfg.nace);
    xml.push_str("/>\n<VetaP");
    for (k, v) in [
        ("c_ufo", cfg.tax_office.as_str()),
        ("c_pracufo", &cfg.tax_office_branch),
        ("dic", &p.dic),
        ("typ_ds", &cfg.legal_form),
        ("jmeno", &cfg.first_name),
        ("prijmeni", &cfg.last_name),
        ("zkrobchjm", &cfg.company_name),
        ("ulice", &cfg.street),
        ("naz_obce", &cfg.city),
        ("psc", &cfg.postal_code),
        ("stat", "ČESKÁ REPUBLIKA"),
    ] {
        attr(&mut xml, k, v);
    }
    xml.push_str("/>\n");
    section(
        &mut xml,
        "Veta1",
        &[
            "obrat23",
            "dan23",
            "obrat5",
            "dan5",
            "p_sl23_e",
            "dan_psl23_e",
            "p_sl5_e",
            "dan_psl5_e",
            "p_sl23_z",
            "dan_psl23_z",
            "p_sl5_z",
            "dan_psl5_z",
        ],
        p,
    );
    section(
        &mut xml,
        "Veta4",
        &[
            "pln23",
            "odp_tuz23_nar",
            "pln5",
            "odp_tuz5_nar",
            "nar_zdp23",
            "od_zdp23",
            "nar_zdp5",
            "od_zdp5",
            "odp_sum_nar",
        ],
        p,
    );
    xml.push_str("<Veta6");
    attr(&mut xml, "dan_zocelk", &p.output.to_string());
    attr(&mut xml, "odp_zocelk", &p.input.to_string());
    let due = p.output - p.input;
    if due >= Decimal::ZERO {
        attr(&mut xml, "dano_da", &due.to_string());
    } else {
        attr(&mut xml, "dano_no", &(-due).to_string());
    }
    xml.push_str("/>\n</DPHDP3>\n</Pisemnost>\n");
    Ok(xml)
}
struct WorkDir(PathBuf);
impl WorkDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "invoice-tax-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir(&path).map_err(error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .map_err(error)?;
        }
        Ok(Self(path))
    }
}
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn validate(xml: &str) -> Result<()> {
    let dir = WorkDir::new()?;
    let schema = dir.0.join("schema.xsd");
    let document = dir.0.join("return.xml");
    std::fs::write(&schema, SCHEMA).map_err(error)?;
    std::fs::write(&document, xml).map_err(error)?;
    let output = Command::new("/usr/bin/xmllint")
        .args(["--nonet", "--noout", "--schema"])
        .arg(schema)
        .arg(document)
        .output()
        .map_err(|_| "Local XSD validation requires /usr/bin/xmllint".to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Official XSD validation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
pub async fn tax_xml(pool: &SqlitePool, period: &str) -> Result<Value> {
    let s = core::snapshot(pool).await?;
    let cfg = config(pool).await?;
    let p = prepare(&s, &cfg, period)?;
    if !p.blockers.is_empty() {
        return Err(p.blockers.join("\n"));
    }
    let xml = render(&cfg, &p, period)?;
    validate(&xml)?;
    Ok(
        json!({"filename":format!("DPHDP3-{period}.xml"),"xml":xml,"validation":"official_xsd_valid","status":"prepared_not_submitted","sha256":hex::encode(Sha256::digest(xml.as_bytes())),"rule_version":RULE_VERSION}),
    )
}
fn signed_bytes(path: &str, xml: &str) -> Result<Vec<u8>> {
    let file = PathBuf::from(path);
    let metadata = std::fs::metadata(&file).map_err(|_| "Signed test file cannot be opened")?;
    if !metadata.is_file() || metadata.len() > 5 * 1024 * 1024 {
        return Err("Provide a DER PKCS#7 signed document no larger than 5 MiB".into());
    }
    let data = std::fs::read(&file).map_err(|_| "Cannot read signed test file")?;
    let dir = WorkDir::new()?;
    let copy = dir.0.join("signed.der");
    let extracted = dir.0.join("content.xml");
    std::fs::write(&copy, &data).map_err(error)?;
    let output = Command::new("/usr/bin/openssl")
        .args(["cms", "-verify", "-inform", "DER", "-noverify", "-in"])
        .arg(copy)
        .arg("-out")
        .arg(&extracted)
        .output()
        .map_err(|_| "OpenSSL is required to inspect the signed test document")?;
    if !output.status.success() {
        return Err("Cannot extract a valid encapsulated CMS document. EPO requires the documented recognized electronic signature.".into());
    }
    if std::fs::read(extracted).map_err(error)? != xml.as_bytes() {
        return Err("The signed document does not exactly match the current XML export. Export and sign the current version.".into());
    }
    Ok(data)
}
fn test_response(body: &str) -> Result<bool> {
    let mut reader = quick_xml::Reader::from_str(body);
    let mut test_marker = false;
    let mut errors = false;
    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(e)) | Ok(quick_xml::events::Event::Empty(e)) => {
                if e.local_name().as_ref() == b"Chyba" {
                    let mut info = false;
                    let mut marker = false;
                    for a in e.attributes() {
                        let a = a.map_err(error)?;
                        let value = a
                            .decode_and_unescape_value(reader.decoder())
                            .map_err(error)?;
                        if a.key.as_ref() == b"Typ" {
                            info = value == "I";
                        }
                        if a.key.as_ref() == b"Zkr" {
                            marker = value == "TEST_REZIM";
                        }
                    }
                    if info && marker {
                        test_marker = true;
                    } else if !info {
                        errors = true;
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(_) => return Err("EPO returned an unreadable validation response".into()),
            _ => {}
        }
    }
    Ok(test_marker && !errors)
}
pub async fn tax_test(pool: &SqlitePool, period: &str) -> Result<Value> {
    if core::snapshot(pool).await?.demo {
        return Err("Live network validation is disabled for the isolated demo database".into());
    }
    let cfg = config(pool).await?;
    if cfg.signed_test_file.trim().is_empty() {
        return Ok(
            json!({"status":"needs_signature","message":"Export the current XML, sign it with a recognized electronic signature, and select the DER PKCS#7 file. The test adapter never files a return."}),
        );
    }
    let generated = tax_xml(pool, period).await?;
    let xml = text(&generated, "xml");
    let bytes = signed_bytes(&cfg.signed_test_file, xml)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot initialize EPO connection")?;
    let response = client
        .post("https://mojedane.gov.cz/dpr/epo_podani?test=1")
        .header("Content-Type", "application/octet-stream")
        .body(bytes)
        .send()
        .await
        .map_err(|_| "EPO test connection failed. No production submission was attempted.")?;
    if !response.status().is_success() {
        return Err(format!(
            "EPO test endpoint returned HTTP {}; no production filing was attempted.",
            response.status().as_u16()
        ));
    }
    if response.content_length().unwrap_or(0) > 2 * 1024 * 1024 {
        return Err("EPO validation response exceeds the supported size".into());
    }
    let body = response
        .text()
        .await
        .map_err(|_| "Cannot read EPO test response")?;
    if body.len() > 2 * 1024 * 1024 {
        return Err("EPO validation response exceeds the supported size".into());
    }
    let passed = test_response(&body)?;
    let status = if passed {
        "test_validated_not_submitted"
    } else {
        "test_failed_not_submitted"
    };
    let detail=json!({"period":period,"status":status,"xml_sha256":generated["sha256"],"rule_version":RULE_VERSION,"response":body}).to_string();
    sqlx::query("INSERT INTO jobs(id,kind,outcome,created_at,detail) VALUES(?,?,?,?,?)")
        .bind(format!(
            "epo-test-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
        .bind("epo_test")
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(detail)
        .execute(pool)
        .await
        .map_err(error)?;
    Ok(
        json!({"status":status,"message":if passed{"EPO test validation passed. Nothing was submitted."}else{"EPO reported validation errors. Nothing was submitted. See the recorded response."},"response":body}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ledger(regime: &str) -> Snapshot {
        serde_json::from_value(json!({
            "business":{"name":"Test only","ico":"00000000","dic":"CZ00000000","registrations":[{"from":"2020-01-01","regime":regime,"dic":"CZ00000000"}]},
            "customers":[],"invoices":[],"recurrences":[],"bank":[],"jobs":[],"exceptions":[],"demo":true,"today":"2026-10-01",
            "purchases":[
                {"id":"p1","supplier":"Domestic fixture","supplier_dic":"CZ00000001","document_number":"TEST-1","supply_date":"2026-09-10","currency":"CZK","base":"1000.00","vat":"210.00","vat_rate":"21","treatment":"domestic_purchase","note":"Test only"},
                {"id":"p2","supplier":"EU fixture","supplier_dic":"DE000000000","document_number":"TEST-2","supply_date":"2026-09-15","currency":"CZK","base":"1000.00","vat":"0","vat_rate":"21","treatment":"foreign_service_eu","note":"Test only"}
            ]
        })).unwrap()
    }
    fn confirmed(s: &Snapshot) -> Config {
        Config {
            tax_office: "451".into(),
            legal_form: "F".into(),
            first_name: "Test".into(),
            last_name: "Only".into(),
            street: "Testovací 1".into(),
            city: "Praha".into(),
            postal_code: "11000".into(),
            records_complete: true,
            confirmed_period: "2026-09".into(),
            confirmed_fingerprint: fingerprint(s, "2026-09").unwrap(),
            ..Config::default()
        }
    }
    #[test]
    fn identified_person_self_assesses_without_deducting() {
        let s = ledger("identified");
        let cfg = confirmed(&s);
        let p = prepare(&s, &cfg, "2026-09").unwrap();
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.output, dec("210").unwrap());
        assert_eq!(p.input, Decimal::ZERO);
        let xml = render(&cfg, &p, "2026-09").unwrap();
        assert!(xml.contains("typ_platce=\"I\""));
        assert!(xml.contains("dan_psl23_e=\"210\""));
        assert!(!xml.contains("<Veta4"));
        validate(&xml).unwrap();
    }
    #[test]
    fn payer_input_uses_full_deduction_column() {
        let s = ledger("payer");
        let cfg = confirmed(&s);
        let p = prepare(&s, &cfg, "2026-09").unwrap();
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.output, dec("210").unwrap());
        assert_eq!(p.input, dec("420").unwrap());
        let xml = render(&cfg, &p, "2026-09").unwrap();
        assert!(xml.contains("odp_tuz23_nar=\"210\""));
        assert!(!xml.contains("odp_tuz23=\""));
        assert!(xml.contains("odp_sum_nar=\"420\""));
        assert!(xml.contains("dano_no=\"210\""));
        validate(&xml).unwrap();
    }
    #[test]
    fn small_identified_purchase_is_not_lost_to_form_rounding() {
        let mut s = ledger("identified");
        s.purchases[1].base = "1.00".into();
        let cfg = confirmed(&s);
        let p = prepare(&s, &cfg, "2026-09").unwrap();
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.output, Decimal::ZERO);
    }
    #[test]
    fn changing_records_invalidates_period_confirmation() {
        let mut s = ledger("payer");
        let cfg = confirmed(&s);
        s.purchases[0].base = "2000".into();
        assert!(prepare(&s, &cfg, "2026-09")
            .unwrap()
            .blockers
            .iter()
            .any(|m| m.contains("Confirm that")));
    }
    #[test]
    fn imported_archive_blocks_vat_export_with_specific_reason() {
        let mut s = ledger("payer");
        s.invoices.push(serde_json::from_value(json!({
            "id":"imported-1","customer_id":"c1","number":"ID-2026-9","status":"archived",
            "issue_date":"2026-09-10","due_date":"2026-10-10","supply_date":"2026-09-10",
            "currency":"CZK","lines":[],"totals":{"base":"100","vat":"21","gross":"121"},
            "regime":null,"seller":null,"customer":null,"issued_at":null,"recurrence_id":null
        })).unwrap());
        let cfg = confirmed(&s);
        let p = prepare(&s, &cfg, "2026-09").unwrap();
        assert!(p.blockers.iter().any(|message| message.contains("Imported historical invoice ID-2026-9")));
        assert_eq!(p.invoice_count, 0);
    }
    #[test]
    fn unknown_treatment_and_fx_block_export() {
        let mut s = ledger("payer");
        s.purchases[0].treatment = "unmapped".into();
        s.purchases[1].currency = "EUR".into();
        let cfg = confirmed(&s);
        let p = prepare(&s, &cfg, "2026-09").unwrap();
        assert!(p.blockers.iter().any(|m| m.contains("not supported")));
        assert!(p.blockers.iter().any(|m| m.contains("conversion")));
    }
    #[test]
    fn period_validation() {
        assert!(period_dates("2026-13").is_err());
        assert!(period_dates("2026-1").is_err());
        assert_eq!(period_dates("2028-02").unwrap().1.to_string(), "2028-02-29");
    }
    #[test]
    fn response_is_never_confused_with_submission() {
        assert!(test_response(
            r#"<Chyby><Chyba Typ="I" Zkr="TEST_REZIM"><Text>Not submitted</Text></Chyba></Chyby>"#
        )
        .unwrap());
        assert!(!test_response(
            r#"<Chyby><Chyba Typ="K" Zkr="BAD"/><Chyba Typ="I" Zkr="TEST_REZIM"/></Chyby>"#
        )
        .unwrap());
        assert!(!test_response("<html>Server error</html>").unwrap());
    }
    #[test]
    fn signed_test_payload_must_match_current_xml_exactly() {
        let dir = WorkDir::new().unwrap();
        let key = dir.0.join("fixture-key.pem");
        let cert = dir.0.join("fixture-cert.pem");
        let source = dir.0.join("source.xml");
        let signed = dir.0.join("signed.der");
        let xml = "<?xml version=\"1.0\"?><Pisemnost/>\n";
        std::fs::write(&source, xml).unwrap();
        let created = Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-subj",
                "/CN=Disposable local test only",
                "-days",
                "1",
                "-keyout",
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap();
        assert!(
            created.status.success(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let result = Command::new("/usr/bin/openssl")
            .args([
                "cms",
                "-sign",
                "-binary",
                "-nodetach",
                "-outform",
                "DER",
                "-in",
            ])
            .arg(&source)
            .arg("-signer")
            .arg(&cert)
            .arg("-inkey")
            .arg(&key)
            .arg("-out")
            .arg(&signed)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(signed_bytes(signed.to_str().unwrap(), xml).is_ok());
        assert!(signed_bytes(signed.to_str().unwrap(), "<Pisemnost/>")
            .unwrap_err()
            .contains("does not exactly match"));
    }
    #[test]
    fn decimal_row_rounding() {
        assert_eq!(crowns(dec("42.50").unwrap()), dec("43").unwrap());
        assert_eq!(crowns(dec("42.49").unwrap()), dec("42").unwrap());
    }
    #[test]
    fn generated_xml_matches_official_schema() {
        let cfg = Config {
            tax_office: "451".into(),
            legal_form: "F".into(),
            first_name: "Test".into(),
            last_name: "A & B".into(),
            street: "Testovací 1".into(),
            city: "Praha".into(),
            postal_code: "11000".into(),
            ..Config::default()
        };
        let mut p = Summary {
            regime: "payer".into(),
            dic: "00000000".into(),
            output: dec("2100").unwrap(),
            input: dec("210").unwrap(),
            ..Summary::default()
        };
        p.add("obrat23", dec("10000").unwrap());
        p.add("dan23", dec("2100").unwrap());
        p.add("pln23", dec("1000").unwrap());
        p.add("odp_tuz23_nar", dec("210").unwrap());
        p.add("odp_sum_nar", dec("210").unwrap());
        let xml = render(&cfg, &p, "2026-09").unwrap();
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("dano_da=\"1890\""));
        validate(&xml).unwrap();
    }
}
