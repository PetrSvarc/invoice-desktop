//! Supporting §7 / Příloha 1 working papers. Never treats an invoice date as a cash receipt.
use crate::core::{BankTx, Invoice, Purchase, Result};
use chrono::{Datelike, NaiveDate};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

const META_PREFIX: &str = "annual_fo_config_";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnnualConfig {
    pub year: i32,
    pub expense_mode: String,
    pub default_category: Option<String>,
    pub bank_allocations: Vec<BankAllocation>,
    pub manual_entries: Vec<ManualEntry>,
    pub reviewed_purchase_ids: Vec<String>,
    /// User verified all receipts of these imported invoices for this particular year.
    pub reviewed_archive_ids: Vec<String>,
    /// Confirms receivables were separately reconciled when non-bank settlements exist.
    pub nonlinked_receivables_reviewed: bool,
    /// Hash of records and classifications at the moment user confirmed completeness.
    pub records_complete_fingerprint: Option<String>,
}
impl Default for AnnualConfig {
    fn default() -> Self {
        Self {
            year: 0,
            expense_mode: "unselected".into(),
            default_category: None,
            bank_allocations: vec![],
            manual_entries: vec![],
            reviewed_purchase_ids: vec![],
            reviewed_archive_ids: vec![],
            nonlinked_receivables_reviewed: false,
            records_complete_fingerprint: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BankAllocation {
    pub bank_id: String,
    pub kind: String, // income, income_refund, expense, exclude
    pub category: String,
    pub amount: String, // positive magnitude in the bank transaction's currency
    pub taxable_czk: String, // explicit CZK value for foreign/unlinked income and every expense
    pub note: String,
    pub purchase_id: String,
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ManualEntry {
    pub id: String,
    pub date: String, // actual receipt/payment date
    pub kind: String,
    pub category: String,
    pub amount: String,
    pub currency: String,
    pub taxable_czk: String,
    pub description: String,
    pub evidence: String,
    pub fx_note: String,
    pub purchase_id: String,
}

fn dec(s: &str) -> std::result::Result<Decimal, String> {
    Decimal::from_str(s).map_err(|_| format!("Neplatná částka: {s}"))
}
fn money(n: Decimal) -> String {
    n.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .to_string()
}
fn issue(code: &str, message: impl Into<String>, source_id: &str) -> Value {
    json!({"code":code,"message":message.into(),"source_id":source_id})
}
fn category_info(s: &str) -> Option<(i64, i64)> {
    match s {
        "trade_80" => Some((80, 1_600_000)),
        "trade_60" => Some((60, 1_200_000)),
        "other_40" => Some((40, 800_000)),
        "rent_30" => Some((30, 600_000)),
        _ => None,
    }
}
fn valid_year(year: i32) -> Result<()> {
    if (2000..=2100).contains(&year) {
        Ok(())
    } else {
        Err("Unsupported year".into())
    }
}
fn validate_config(config: &AnnualConfig, year: i32) -> Result<()> {
    valid_year(year)?;
    if config.year != year {
        return Err("Configuration year does not match".into());
    }
    if !["unselected", "percentage", "actual"].contains(&config.expense_mode.as_str()) {
        return Err("Unknown expense mode".into());
    }
    if let Some(c) = &config.default_category {
        if category_info(c).is_none() {
            return Err("Unknown default income category".into());
        }
    }
    let mut manual_ids = BTreeSet::new();
    for m in &config.manual_entries {
        if m.id.trim().is_empty() || !manual_ids.insert(&m.id) {
            return Err("Manual entries need distinct IDs".into());
        }
        let d = NaiveDate::parse_from_str(&m.date, "%Y-%m-%d").map_err(|e| e.to_string())?;
        if d.year() != year {
            return Err("Manual payment date is outside the selected year".into());
        }
        if !["income", "income_refund", "expense"].contains(&m.kind.as_str()) {
            return Err("Unknown manual entry kind".into());
        }
        if dec(&m.amount)? <= Decimal::ZERO || dec(&m.taxable_czk)? < Decimal::ZERO {
            return Err("Manual amounts must be positive".into());
        }
        if m.currency == "CZK" && dec(&m.taxable_czk)? > dec(&m.amount)? {
            return Err("Manual CZK tax amount exceeds paid amount".into());
        }
        if m.currency != "CZK" && m.fx_note.trim().is_empty() {
            return Err("Foreign-currency manual entry needs conversion evidence".into());
        }
        if m.currency.len() != 3 || !m.currency.chars().all(|c| c.is_ascii_uppercase()) {
            return Err("Currency must be a three-letter uppercase code".into());
        }
        if m.currency.trim().is_empty() || m.evidence.trim().is_empty() {
            return Err("Manual entries need currency and payment evidence".into());
        }
        if config.expense_mode == "percentage"
            && m.kind != "expense"
            && category_info(&m.category).is_none()
        {
            return Err("Manual income needs a §7 category".into());
        }
    }
    for a in &config.bank_allocations {
        if a.bank_id.trim().is_empty()
            || !["income", "income_refund", "expense", "exclude"].contains(&a.kind.as_str())
        {
            return Err("Invalid bank allocation".into());
        }
        if dec(&a.amount)? <= Decimal::ZERO {
            return Err("Allocation amount must be positive".into());
        }
        if !a.taxable_czk.is_empty() && dec(&a.taxable_czk)? < Decimal::ZERO {
            return Err("Taxable CZK amount cannot be negative".into());
        }
        if config.expense_mode == "percentage"
            && matches!(a.kind.as_str(), "income" | "income_refund")
            && category_info(&a.category).is_none()
        {
            return Err("Income allocation needs a §7 category".into());
        }
        if a.kind == "expense" && (a.taxable_czk.is_empty() || a.note.trim().is_empty()) {
            return Err(
                "Expense allocation needs paid deductible CZK amount and tax classification note"
                    .into(),
            );
        }
    }
    Ok(())
}

pub async fn annual_config(pool: &SqlitePool, year: i32) -> Result<Value> {
    valid_year(year)?;
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
        .bind(format!("{META_PREFIX}{year}"))
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    let config = if let Some(s) = raw {
        serde_json::from_str::<AnnualConfig>(&s).map_err(|e| e.to_string())?
    } else {
        AnnualConfig {
            year,
            ..Default::default()
        }
    };
    serde_json::to_value(config).map_err(|e| e.to_string())
}
pub async fn annual_save_config(pool: &SqlitePool, year: i32, config: Value) -> Result<Value> {
    let confirm = config
        .get("confirm_records_complete")
        .and_then(Value::as_bool)
        == Some(true);
    let mut config: AnnualConfig = serde_json::from_value(config).map_err(|e| e.to_string())?;
    validate_config(&config, year)?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    if confirm {
        let invoices: Vec<Invoice> = table_tx(&mut tx, "invoices").await?;
        let bank: Vec<BankTx> = table_tx(&mut tx, "bank_transactions").await?;
        let purchases: Vec<Purchase> = table_tx(&mut tx, "purchases").await?;
        let meta = fingerprint_meta(&mut tx, year).await?;
        config.records_complete_fingerprint = Some(make_fingerprint(
            &config, &invoices, &bank, &purchases, &meta,
        )?);
    }
    let value = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO meta(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value")
        .bind(format!("{META_PREFIX}{year}"))
        .bind(value).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    serde_json::to_value(config).map_err(|e| e.to_string())
}
async fn table_tx<T: for<'a> Deserialize<'a>>(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<Vec<T>> {
    let sql = match table {
        "invoices" => "SELECT data FROM invoices ORDER BY id",
        "bank_transactions" => "SELECT data FROM bank_transactions ORDER BY id",
        "purchases" => "SELECT data FROM purchases ORDER BY id",
        _ => return Err("Unsupported table".into()),
    };
    let raw: Vec<String> = sqlx::query_scalar(sql)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    raw.iter()
        .map(|r| serde_json::from_str(r).map_err(|e| e.to_string()))
        .collect()
}
async fn fingerprint_meta(
    tx: &mut Transaction<'_, Sqlite>,
    year: i32,
) -> Result<Vec<(String, String)>> {
    let all: Vec<(String, String)> = sqlx::query_as("SELECT key,value FROM meta ORDER BY key")
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let mut included = Vec::new();
    for (key, value) in all {
        if key.starts_with(META_PREFIX) {
            let mut previous: AnnualConfig = serde_json::from_str(&value).map_err(|e| e.to_string())?;
            if previous.year >= year { continue; }
            previous.records_complete_fingerprint = None;
            included.push((key, serde_json::to_string(&previous).map_err(|e| e.to_string())?));
        } else {
            included.push((key, value));
        }
    }
    Ok(included)
}
pub async fn annual_summary(pool: &SqlitePool, year: i32) -> Result<Value> {
    valid_year(year)?;
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
        .bind(format!("{META_PREFIX}{year}"))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let c: AnnualConfig = if let Some(raw) = raw {
        serde_json::from_str(&raw).map_err(|e| e.to_string())?
    } else {
        AnnualConfig {
            year,
            ..Default::default()
        }
    };
    let invoices: Vec<Invoice> = table_tx(&mut tx, "invoices").await?;
    let bank: Vec<BankTx> = table_tx(&mut tx, "bank_transactions").await?;
    let purchases: Vec<Purchase> = table_tx(&mut tx, "purchases").await?;
    let meta = fingerprint_meta(&mut tx, year).await?;
    let bank_by_id: BTreeMap<&str, &BankTx> = bank.iter().map(|b| (b.id.as_str(), b)).collect();
    let mut prior_override_invoice_ids = BTreeSet::new();
    for (key, raw) in &meta {
        if !key.starts_with(META_PREFIX) { continue; }
        let previous: AnnualConfig = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        for a in previous.bank_allocations {
            if a.kind != "income" || !a.taxable_czk.is_empty() {
                if let Some(invoice_id) = bank_by_id.get(a.bank_id.as_str()).and_then(|b| b.invoice_id.as_ref()) {
                    prior_override_invoice_ids.insert(invoice_id.clone());
                }
            }
        }
    }
    let mut summary = calculate_inner(year, &c, &invoices, &bank, &purchases, &prior_override_invoice_ids)?;
    let fingerprint = make_fingerprint(&c, &invoices, &bank, &purchases, &meta)?;
    tx.commit().await.map_err(|e| e.to_string())?;
    let records_complete = c.records_complete_fingerprint.as_deref() == Some(&fingerprint);
    summary["current_fingerprint"] = json!(fingerprint);
    summary["records_complete"] = json!(records_complete);
    if !records_complete {
        summary["complete"] = json!(false);
        summary["issues"]
            .as_array_mut()
            .ok_or("Missing issues")?
            .push(issue(
                "records_not_confirmed",
                "Potvrďte úplnost příjmů a výdajů po kontrole všech podkladů.",
                "",
            ));
    }
    if year >= chrono::Local::now().date_naive().year() {
        summary["provisional"] = json!(true);
        summary["complete"] = json!(false);
        summary["issues"]
            .as_array_mut()
            .ok_or("Missing issues")?
            .push(issue(
                "year_not_finished",
                "Rok ještě neskončil; souhrn je průběžný.",
                "",
            ));
    } else {
        summary["provisional"] = json!(false);
    }
    Ok(summary)
}
fn make_fingerprint(
    c: &AnnualConfig,
    invoices: &[Invoice],
    bank: &[BankTx],
    purchases: &[Purchase],
    meta: &[(String, String)],
) -> Result<String> {
    let mut stable_config = c.clone();
    stable_config.records_complete_fingerprint = None;
    let encoded = serde_json::to_vec(&(invoices, bank, purchases, meta, &stable_config))
        .map_err(|e| e.to_string())?;
    Ok(hex::encode(Sha256::digest(encoded)))
}
pub async fn annual_csv(pool: &SqlitePool, year: i32) -> Result<String> {
    let s = annual_summary(pool, year).await?;
    let mut w = csv::WriterBuilder::new().flexible(true).from_writer(vec![]);
    w.write_record(["PODKLAD §7 / PŘÍLOHA 1 — NEJDE O DAŇOVÉ PŘIZNÁNÍ"])
        .map_err(|e| e.to_string())?;
    for (label, key) in [
        ("Rok", "year"),
        ("Režim výdajů", "expense_mode"),
        ("Příjmy Kč", "income_czk"),
        ("Výdaje Kč", "expenses_czk"),
        ("Rozdíl §7 Kč", "section7_difference_czk"),
        ("Úplné", "complete"),
        ("Průběžné", "provisional"),
    ] {
        w.write_record([
            label,
            s[key]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| s[key].to_string())
                .as_str(),
        ])
        .map_err(|e| e.to_string())?;
    }
    for g in s["percentage_groups"]
        .as_array()
        .ok_or("Missing percentage groups")?
    {
        w.write_record([
            "Paušální skupina",
            g["category"].as_str().unwrap_or(""),
            g["rate"].to_string().as_str(),
            g["income_czk"].as_str().unwrap_or(""),
            g["expense_czk"].as_str().unwrap_or(""),
            g["cap_czk"].as_str().unwrap_or(""),
        ])
        .map_err(|e| e.to_string())?;
    }
    for i in s["issues"].as_array().ok_or("Missing issues")? {
        w.write_record([
            "Nedokončeno",
            i["code"].as_str().unwrap_or(""),
            safe_csv(i["message"].as_str().unwrap_or("")).as_str(),
            safe_csv(i["source_id"].as_str().unwrap_or("")).as_str(),
        ])
        .map_err(|e| e.to_string())?;
    }
    w.write_record(std::iter::empty::<&str>())
        .map_err(|e| e.to_string())?;
    w.write_record([
        "year",
        "kind",
        "date",
        "source",
        "source_id",
        "invoice_id",
        "purchase_id",
        "category",
        "currency",
        "amount",
        "taxable_czk",
        "note",
        "status",
    ])
    .map_err(|e| e.to_string())?;
    for row in s["rows"].as_array().ok_or("Missing rows")? {
        let fields = [
            "year",
            "kind",
            "date",
            "source",
            "source_id",
            "invoice_id",
            "purchase_id",
            "category",
            "currency",
            "amount",
            "taxable_czk",
            "note",
            "status",
        ];
        let record: Vec<String> = fields
            .iter()
            .map(|k| {
                let value = match &row[*k] {
                    Value::String(s) => s.clone(),
                    Value::Null => String::new(),
                    v => v.to_string(),
                };
                if ["source_id", "invoice_id", "purchase_id", "category", "note"].contains(k) {
                    safe_csv(&value)
                } else {
                    value
                }
            })
            .collect();
        w.write_record(record).map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    String::from_utf8(w.into_inner().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}
fn safe_csv(s: &str) -> String {
    if s.trim_start().starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
fn calculate(
    year: i32,
    config: &AnnualConfig,
    invoices: &[Invoice],
    bank: &[BankTx],
    purchases: &[Purchase],
) -> Result<Value> {
    calculate_inner(year, config, invoices, bank, purchases, &BTreeSet::new())
}
fn calculate_inner(
    year: i32,
    config: &AnnualConfig,
    invoices: &[Invoice],
    bank: &[BankTx],
    purchases: &[Purchase],
    prior_override_invoice_ids: &BTreeSet<String>,
) -> Result<Value> {
    validate_config(config, year)?;
    let end = NaiveDate::from_ymd_opt(year, 12, 31).ok_or("Invalid year")?;
    let inv_by_id: BTreeMap<&str, &Invoice> = invoices.iter().map(|i| (i.id.as_str(), i)).collect();
    let bank_by_id: BTreeMap<&str, &BankTx> = bank.iter().map(|b| (b.id.as_str(), b)).collect();
    let purchase_ids: BTreeSet<&str> = purchases.iter().map(|p| p.id.as_str()).collect();
    let mut issues = Vec::<Value>::new();
    if !(2025..=2026).contains(&year) {
        issues.push(issue(
            "year_rules_unverified",
            "Pravidla výdajových paušálů pro tento rok nejsou v aplikaci ověřena.",
            "",
        ));
    }
    if config.expense_mode == "unselected" {
        issues.push(issue(
            "expense_mode_missing",
            "Zvolte skutečné nebo procentní výdaje pro tento rok.",
            "",
        ));
    }
    let mut paid_through_end: BTreeMap<&str, Decimal> = BTreeMap::new();
    for b in bank {
        if let Some(id) = b.invoice_id.as_deref() {
            match (
                NaiveDate::parse_from_str(&b.booked, "%Y-%m-%d"),
                dec(&b.amount),
            ) {
                (Ok(d), Ok(amount)) if d > end => {
                    let _ = amount;
                }
                (Ok(_), Ok(amount))
                    if b.currency
                        == inv_by_id.get(id).map(|i| i.currency.as_str()).unwrap_or("") =>
                {
                    *paid_through_end.entry(id).or_default() += amount;
                }
                _ => issues.push(issue(
                    "linked_payment_invalid",
                    "Propojená úhrada má chybné datum, částku nebo měnu.",
                    &b.id,
                )),
            }
        }
    }
    let mut outstanding = Vec::new();
    let mut archived_invoices = Vec::new();
    let mut overpaid_invoices = BTreeSet::new();
    let mut rows = Vec::new();
    for i in invoices {
        let Ok(issued) = NaiveDate::parse_from_str(&i.issue_date, "%Y-%m-%d") else {
            issues.push(issue(
                "invoice_date_invalid",
                "Faktura má neplatné datum vystavení.",
                &i.id,
            ));
            continue;
        };
        if issued > end {
            continue;
        }
        if i.status == "archived" {
            if dec(&i.totals.gross)
                .map(|n| n < Decimal::ZERO)
                .unwrap_or(true)
            {
                issues.push(issue(
                    "archive_amount_invalid",
                    "Archivní faktura má neplatnou celkovou částku.",
                    &i.id,
                ));
            }
            let reviewed = config.reviewed_archive_ids.contains(&i.id);
            archived_invoices.push(json!({"invoice_id":i.id,"number":i.number,"issue_date":i.issue_date,"currency":i.currency,"gross":i.totals.gross,"payment_state":"unknown","receipts_reviewed_for_year":reviewed}));
            if !reviewed {
                issues.push(issue(
                    "archive_payment_unknown",
                    "Archivní faktura nemá úplnou historii úhrad a DPH; příjmy vyžadují kontrolu.",
                    &i.id,
                ));
            }
            rows.push(json!({"year":year,"kind":"archived_invoice","date":i.issue_date,"source":"invoice","source_id":i.id,"invoice_id":i.id,"purchase_id":"","category":"","currency":i.currency,"amount":i.totals.gross,"taxable_czk":"","note":if reviewed {"Příjmy v roce byly samostatně odsouhlaseny; stav úhrady není v aplikaci doložen"} else {"Úhrada neznámá"},"status":if reviewed {"supporting"} else {"unresolved"}}));
            continue;
        }
        if i.status != "issued" {
            continue;
        }
        let Ok(gross) = dec(&i.totals.gross) else {
            issues.push(issue(
                "invoice_amount_invalid",
                "Faktura má neplatnou částku.",
                &i.id,
            ));
            continue;
        };
        let paid = paid_through_end
            .get(i.id.as_str())
            .copied()
            .unwrap_or_default();
        if paid > gross {
            issues.push(issue(
                "invoice_overpaid",
                "Úhrady převyšují fakturu; přeplatek nebo vrácení vyžaduje posouzení.",
                &i.id,
            ));
            overpaid_invoices.insert(i.id.as_str());
        }
        if paid < Decimal::ZERO {
            issues.push(issue(
                "invoice_negative_paid",
                "Záporný součet propojených úhrad vyžaduje kontrolu.",
                &i.id,
            ));
        }
        let remaining = (gross - paid).max(Decimal::ZERO);
        if remaining > Decimal::ZERO {
            outstanding.push(json!({"invoice_id":i.id,"number":i.number,"currency":i.currency,"gross":i.totals.gross,"paid_through_year_end":money(paid),"outstanding":money(remaining),"basis":"linked_bank_only"}));
        }
        rows.push(json!({"year":year,"kind":"invoice_receivable","date":i.issue_date,"source":"invoice","source_id":i.id,"invoice_id":i.id,"purchase_id":"","category":"","currency":i.currency,"amount":money(remaining),"taxable_czk":"","note":"Zůstatek podle propojených bankovních úhrad k 31. 12.; jiné úhrady nezohledněny","status":"supporting"}));
    }
    let mut allocations: BTreeMap<&str, Vec<&BankAllocation>> = BTreeMap::new();
    for a in &config.bank_allocations {
        if !bank_by_id.contains_key(a.bank_id.as_str()) {
            issues.push(issue(
                "bank_allocation_orphan",
                "Klasifikace odkazuje na chybějící bankovní pohyb.",
                &a.bank_id,
            ));
        }
        allocations.entry(&a.bank_id).or_default().push(a);
    }
    let mut income = Decimal::ZERO;
    let mut actual_expenses = Decimal::ZERO;
    let mut groups: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut referenced_purchases = BTreeSet::new();
    let mut has_nonlinked_receipts = config
        .manual_entries
        .iter()
        .any(|m| matches!(m.kind.as_str(), "income" | "income_refund"));
    let mut ordered_bank: Vec<&BankTx> = bank.iter().collect();
    ordered_bank.sort_by(|a, b| a.booked.cmp(&b.booked).then_with(|| a.id.cmp(&b.id)));
    let year_start = NaiveDate::from_ymd_opt(year, 1, 1).ok_or("Invalid year")?;
    let mut payer_progress: BTreeMap<&str, (Decimal, Decimal)> = BTreeMap::new();
    let mut prior_uncertain_invoice_ids = prior_override_invoice_ids.clone();
    for b in &ordered_bank {
        let Some(id) = b.invoice_id.as_deref() else { continue; };
        let Some(i) = inv_by_id.get(id).copied() else { continue; };
        if i.status != "issued" || i.regime.as_deref() != Some("payer") { continue; }
        let Ok(booked) = NaiveDate::parse_from_str(&b.booked, "%Y-%m-%d") else { continue; };
        if booked >= year_start { continue; }
        let Ok(payment) = dec(&b.amount) else { prior_uncertain_invoice_ids.insert(id.to_string()); continue; };
        if payment <= Decimal::ZERO || b.currency != "CZK" || i.currency != "CZK" {
            prior_uncertain_invoice_ids.insert(id.to_string());
            continue;
        }
        let (Ok(gross),Ok(base)) = (dec(&i.totals.gross),dec(&i.totals.base)) else {
            prior_uncertain_invoice_ids.insert(id.to_string()); continue;
        };
        if gross <= Decimal::ZERO || base < Decimal::ZERO || base > gross {
            prior_uncertain_invoice_ids.insert(id.to_string()); continue;
        }
        let progress = payer_progress.entry(id).or_default();
        let next_gross = progress.0 + payment;
        if next_gross > gross { prior_uncertain_invoice_ids.insert(id.to_string()); continue; }
        *progress = (next_gross, (next_gross * base / gross).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero));
    }
    for b in ordered_bank {
        let booked = match NaiveDate::parse_from_str(&b.booked, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                issues.push(issue(
                    "bank_date_invalid",
                    "Bankovní pohyb má chybné datum.",
                    &b.id,
                ));
                continue;
            }
        };
        if booked.year() != year {
            continue;
        }
        let amount = match dec(&b.amount) {
            Ok(a) if a != Decimal::ZERO => a,
            _ => {
                issues.push(issue(
                    "bank_amount_invalid",
                    "Bankovní pohyb má nulovou nebo neplatnou částku.",
                    &b.id,
                ));
                continue;
            }
        };
        let abs = amount.abs();
        let mut parts = allocations.get(b.id.as_str()).cloned().unwrap_or_default();
        let invoice = b
            .invoice_id
            .as_deref()
            .and_then(|id| inv_by_id.get(id).copied());
        if invoice.map(|i| i.regime.as_deref() == Some("payer") && prior_uncertain_invoice_ids.contains(&i.id)).unwrap_or(false) {
            issues.push(issue("prior_payer_receipt_unresolved", "Starší úhrada, vrácení nebo ruční daňová úprava brání automatickému rozdělení DPH přes roky.", &b.id));
            rows.push(json!({"year":year,"kind":"prior_payer_receipt_unresolved","date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":"","category":"","currency":b.currency,"amount":b.amount,"taxable_czk":"","note":"Ověřte starší úhrady a daňové úpravy","status":"unresolved"}));
            continue;
        }
        if invoice.is_none()
            && parts
                .iter()
                .any(|a| matches!(a.kind.as_str(), "income" | "income_refund"))
        {
            has_nonlinked_receipts = true;
        }
        if b.invoice_id.is_some() && invoice.is_none() {
            issues.push(issue(
                "linked_invoice_missing",
                "Úhrada odkazuje na chybějící fakturu.",
                &b.id,
            ));
        }
        if invoice.map(|i| i.currency != b.currency).unwrap_or(false) {
            issues.push(issue(
                "linked_currency_mismatch",
                "Měna propojené úhrady se liší od měny faktury.",
                &b.id,
            ));
            rows.push(json!({"year":year,"kind":"currency_mismatch","date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":"","category":"","currency":b.currency,"amount":b.amount,"taxable_czk":"","note":"Měna faktury a platby nesouhlasí","status":"unresolved"}));
            continue;
        }
        if b.invoice_id
            .as_deref()
            .map(|id| overpaid_invoices.contains(id))
            .unwrap_or(false)
        {
            rows.push(json!({"year":year,"kind":"overpaid_invoice_receipt","date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":"","category":"","currency":b.currency,"amount":b.amount,"taxable_czk":"","note":"Úhrada vyžaduje rozdělení přeplatku/vrácení","status":"unresolved"}));
            continue;
        }
        if parts.is_empty()
            && amount > Decimal::ZERO
            && invoice.map(|i| i.status == "issued").unwrap_or(false)
        {
            // Generated only for this calculation; a selected default is a user assertion about activity.
            let auto = BankAllocation {
                bank_id: b.id.clone(),
                kind: "income".into(),
                category: config.default_category.clone().unwrap_or_default(),
                amount: money(abs),
                ..Default::default()
            };
            // Process the generated allocation with the same checks as explicit entries.
            let value = evaluate_bank_part(
                year,
                b,
                invoice,
                &auto,
                abs,
                config.expense_mode == "percentage",
                &mut payer_progress,
                &mut issues,
                &mut rows,
            )?;
            if let Some(n) = value {
                income += n;
                if !auto.category.is_empty() {
                    *groups.entry(auto.category).or_default() += n;
                }
            }
            continue;
        }
        if parts.is_empty() {
            issues.push(issue(
                "bank_unreviewed",
                "Bankovní pohyb nemá klasifikaci; nelze určit vztah k §7.",
                &b.id,
            ));
            rows.push(json!({"year":year,"kind":"bank_unreviewed","date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":"","category":"","currency":b.currency,"amount":b.amount,"taxable_czk":"","note":b.description,"status":"unresolved"}));
            continue;
        }
        let sum: Decimal = parts
            .iter()
            .map(|a| dec(&a.amount).unwrap_or_default())
            .sum();
        if sum != abs {
            issues.push(issue(
                "bank_allocation_mismatch",
                "Rozdělení bankovního pohybu se nerovná jeho částce.",
                &b.id,
            ));
            rows.push(json!({"year":year,"kind":"bank_allocation_mismatch","date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":"","category":"","currency":b.currency,"amount":b.amount,"taxable_czk":"","note":b.description,"status":"unresolved"}));
            continue;
        }
        for a in parts.drain(..) {
            if !a.purchase_id.is_empty() {
                if !purchase_ids.contains(a.purchase_id.as_str()) {
                    issues.push(issue("purchase_reference_missing", "Úhrada odkazuje na chybějící přijatý doklad.", &a.purchase_id));
                }
                referenced_purchases.insert(a.purchase_id.as_str());
            }
            let part_amount = dec(&a.amount)?;
            let result = evaluate_bank_part(
                year,
                b,
                invoice,
                a,
                part_amount,
                config.expense_mode == "percentage",
                &mut payer_progress,
                &mut issues,
                &mut rows,
            )?;
            if let Some(n) = result {
                match a.kind.as_str() {
                    "income" | "income_refund" => {
                        income += n;
                        if !a.category.is_empty() {
                            *groups.entry(a.category.clone()).or_default() += n;
                        }
                    }
                    "expense" => actual_expenses += n,
                    _ => {}
                }
            }
        }
    }
    let mut seen_manual = BTreeSet::new();
    for m in &config.manual_entries {
        if !seen_manual.insert(&m.id) {
            continue;
        }
        if !m.purchase_id.is_empty() && m.kind == "expense" {
            if !purchase_ids.contains(m.purchase_id.as_str()) {
                issues.push(issue("purchase_reference_missing", "Ruční úhrada odkazuje na chybějící přijatý doklad.", &m.purchase_id));
            }
            referenced_purchases.insert(m.purchase_id.as_str());
        }
        let n = dec(&m.taxable_czk)?;
        let signed = if m.kind == "income_refund" { -n } else { n };
        match m.kind.as_str() {
            "income" | "income_refund" => {
                income += signed;
                if !m.category.is_empty() {
                    *groups.entry(m.category.clone()).or_default() += signed;
                }
            }
            "expense" => actual_expenses += n,
            _ => {}
        }
        rows.push(json!({"year":year,"kind":m.kind,"date":m.date,"source":"manual","source_id":m.id,"invoice_id":"","purchase_id":m.purchase_id,"category":m.category,"currency":m.currency,"amount":m.amount,"taxable_czk":money(signed),"note":format!("{} | důkaz: {} | kurz: {}",m.description,m.evidence,m.fx_note),"status":"included"}));
    }
    if config.expense_mode == "actual" {
        let reviewed: BTreeSet<&str> = config
            .reviewed_purchase_ids
            .iter()
            .map(String::as_str)
            .collect();
        for p in purchases {
            if NaiveDate::parse_from_str(&p.supply_date, "%Y-%m-%d")
                .map(|d| d.year() == year)
                .unwrap_or(false)
                && !referenced_purchases.contains(p.id.as_str())
                && !reviewed.contains(p.id.as_str())
            {
                issues.push(issue(
                    "purchase_payment_unknown",
                    "Přijatý doklad nemá potvrzenou úhradu a daňové zařazení.",
                    &p.id,
                ));
            }
        }
    }
    if has_nonlinked_receipts && !config.nonlinked_receivables_reviewed {
        issues.push(issue("nonlinked_receivables_review", "Ruční nebo nepropojené úhrady mohou změnit pohledávky; odsouhlaste je mimo bankovní vazby.", ""));
    }
    let mut percentage_groups = Vec::new();
    let mut percentage_expenses = Decimal::ZERO;
    for (category, amount) in groups {
        if amount < Decimal::ZERO {
            issues.push(issue(
                "negative_category_income",
                "Vrácené příjmy převyšují příjmy v kategorii.",
                &category,
            ));
        }
        if let Some((rate, cap)) = category_info(&category) {
            let cap_dec = Decimal::from(cap);
            let expense =
                (amount.max(Decimal::ZERO) * Decimal::from(rate) / Decimal::from(100)).min(cap_dec);
            percentage_expenses += expense;
            percentage_groups.push(json!({"category":category,"rate":rate,"income_czk":money(amount),"expense_czk":money(expense),"cap_czk":money(cap_dec)}));
        }
    }
    let expenses = if config.expense_mode == "percentage" {
        percentage_expenses
    } else {
        actual_expenses
    };
    rows.sort_by(|a, b| {
        a["date"]
            .as_str()
            .cmp(&b["date"].as_str())
            .then_with(|| a["source_id"].as_str().cmp(&b["source_id"].as_str()))
    });
    Ok(
        json!({"year":year,"complete":issues.is_empty(),"ready_for_filing":false,"expense_mode":config.expense_mode,
        "income_czk":money(income),"expenses_czk":money(expenses),"section7_difference_czk":money(income-expenses),
        "percentage_groups":percentage_groups,"outstanding":outstanding,"outstanding_basis":"linked_bank_only","archived_invoices":archived_invoices,"rows":rows,"issues":issues,
        "note":"Podklad pro §7 / Přílohu č. 1; nejedná se o hotové přiznání, zálohy ani sociální/zdravotní přehled."}),
    )
}

fn evaluate_bank_part<'a>(
    year: i32,
    b: &BankTx,
    invoice: Option<&'a Invoice>,
    a: &BankAllocation,
    part: Decimal,
    require_category: bool,
    payer_progress: &mut BTreeMap<&'a str, (Decimal, Decimal)>,
    issues: &mut Vec<Value>,
    rows: &mut Vec<Value>,
) -> Result<Option<Decimal>> {
    let mut status = "included";
    let mut taxable: Option<Decimal> = None;
    let positive = dec(&b.amount)? > Decimal::ZERO;
    let proper_sign = match a.kind.as_str() {
        "income" => positive,
        "income_refund" | "expense" => !positive,
        _ => true,
    };
    if !proper_sign {
        issues.push(issue(
            "classification_sign",
            "Klasifikace neodpovídá směru bankovního pohybu.",
            &b.id,
        ));
        status = "unresolved";
    }
    if a.kind == "exclude" {
        if a.note.trim().is_empty() {
            issues.push(issue(
                "exclusion_reason_missing",
                "Vyloučený pohyb potřebuje důvod.",
                &b.id,
            ));
            status = "unresolved";
        }
    } else if matches!(a.kind.as_str(), "income" | "income_refund") {
        if require_category && category_info(&a.category).is_none() {
            issues.push(issue(
                "income_category_missing",
                "Příjem potřebuje výdajovou kategorii §7.",
                &b.id,
            ));
            status = "unresolved";
        }
        if !a.taxable_czk.is_empty() {
            taxable = Some(dec(&a.taxable_czk)?);
        } else if b.currency != "CZK" {
            issues.push(issue(
                "fx_conversion_missing",
                "Cizoměnová úhrada potřebuje doloženou hodnotu v Kč.",
                &b.id,
            ));
            status = "unresolved";
        } else if let Some(i) = invoice {
            if i.status != "issued" || ![Some("payer"),Some("nonpayer"),Some("identified")].contains(&i.regime.as_deref()) {
                issues.push(issue("historical_vat_unknown", "Historický nebo rozpracovaný doklad potřebuje ověřenou hodnotu příjmu bez DPH.", &b.id));
                status = "unresolved";
            } else {
                let gross = dec(&i.totals.gross)?;
                let base = dec(&i.totals.base)?;
                if gross <= Decimal::ZERO || base < Decimal::ZERO || base > gross {
                    issues.push(issue(
                        "invoice_vat_invalid",
                        "Nelze určit základ bez DPH z faktury.",
                        &i.id,
                    ));
                    status = "unresolved";
                } else {
                    taxable = Some(
                        if i.regime.as_deref() == Some("payer") && a.kind == "income" {
                            let progress = payer_progress.entry(i.id.as_str()).or_default();
                            let next_gross = progress.0 + part;
                            let next_taxable = (next_gross * base / gross)
                                .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
                            let value = next_taxable - progress.1;
                            *progress = (next_gross, next_taxable);
                            value
                        } else {
                            part
                        },
                    );
                }
            }
        } else {
            issues.push(issue(
                "unlinked_taxable_missing",
                "Nepropojený příjem potřebuje potvrzenou hodnotu v Kč.",
                &b.id,
            ));
            status = "unresolved";
        }
        if a.kind == "income_refund" && a.note.trim().is_empty() {
            issues.push(issue(
                "refund_reason_missing",
                "Vrácená platba potřebuje důvod a vazbu na původní příjem.",
                &b.id,
            ));
            status = "unresolved";
        }
        if a.kind == "income_refund" && a.taxable_czk.is_empty() {
            issues.push(issue(
                "refund_taxable_missing",
                "Vrácená úhrada potřebuje ověřenou hodnotu původního příjmu v Kč.",
                &b.id,
            ));
            status = "unresolved";
        }
        if ((!a.taxable_czk.is_empty()
            && invoice
                .map(|i| {
                    i.regime.as_deref() == Some("payer")
                    || i.status != "issued"
                    || ![Some("payer"),Some("nonpayer"),Some("identified")].contains(&i.regime.as_deref())
                })
                .unwrap_or(false))
            || b.currency != "CZK"
            || invoice.is_none())
            && a.note.trim().is_empty()
        {
            issues.push(issue(
                "income_evidence_missing",
                "Ručně stanovená hodnota příjmu potřebuje vysvětlení a doklad.",
                &b.id,
            ));
            status = "unresolved";
        }
    } else if a.kind == "expense" {
        taxable = Some(dec(&a.taxable_czk)?);
        if a.note.trim().is_empty() {
            status = "unresolved";
        }
    }
    if let Some(t) = taxable {
        if t < Decimal::ZERO || (b.currency == "CZK" && t > part) {
            issues.push(issue(
                "taxable_amount_invalid",
                "Daňová částka nesmí převyšovat zaplacenou korunovou částku.",
                &b.id,
            ));
            status = "unresolved";
        }
    }
    let signed = taxable.map(|x| if a.kind == "income_refund" { -x } else { x });
    rows.push(json!({"year":year,"kind":a.kind,"date":b.booked,"source":"bank","source_id":b.id,"invoice_id":b.invoice_id,"purchase_id":a.purchase_id,
        "category":a.category,"currency":b.currency,"amount":money(part),"taxable_czk":signed.map(money),"note":if a.note.is_empty() {b.description.as_str()} else {a.note.as_str()},"status":status}));
    Ok(if status == "included" { signed } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Totals;

    fn invoice(gross: &str, base: &str, currency: &str, regime: &str) -> Invoice {
        Invoice {
            id: "inv1".into(),
            customer_id: "cust".into(),
            number: Some("2025-1".into()),
            status: "issued".into(),
            issue_date: "2025-12-20".into(),
            due_date: "2026-01-20".into(),
            supply_date: "2025-12-20".into(),
            currency: currency.into(),
            lines: vec![],
            totals: Totals {
                base: base.into(),
                vat: money(dec(gross).unwrap() - dec(base).unwrap()),
                gross: gross.into(),
            },
            regime: Some(regime.into()),
            seller: None,
            customer: None,
            issued_at: None,
            recurrence_id: None,
            variable_symbol: String::new(),
            constant_symbol: String::new(),
            specific_symbol: String::new(),
        }
    }
    fn bank(id: &str, booked: &str, amount: &str, currency: &str, linked: bool) -> BankTx {
        BankTx {
            id: id.into(),
            source: "test".into(),
            external_id: id.into(),
            booked: booked.into(),
            amount: amount.into(),
            currency: currency.into(),
            variable_symbol: String::new(),
            description: "payment".into(),
            invoice_id: if linked { Some("inv1".into()) } else { None },
        }
    }
    fn config(year: i32, mode: &str) -> AnnualConfig {
        AnnualConfig {
            year,
            expense_mode: mode.into(),
            default_category: Some("trade_60".into()),
            ..Default::default()
        }
    }
    fn codes(s: &Value) -> Vec<&str> {
        s["issues"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x["code"].as_str())
            .collect()
    }
    #[test]
    fn cross_year_payment_is_only_next_year_income_and_prior_year_receivable() {
        let i = invoice("121.00", "100.00", "CZK", "payer");
        let b = bank("b1", "2026-01-05", "121.00", "CZK", true);
        let prior = calculate(
            2025,
            &config(2025, "percentage"),
            &[i.clone()],
            &[b.clone()],
            &[],
        )
        .unwrap();
        assert_eq!(prior["income_czk"], "0");
        assert_eq!(prior["outstanding"][0]["outstanding"], "121.00");
        assert!(!codes(&prior).contains(&"linked_payment_invalid"));
        let next = calculate(2026, &config(2026, "percentage"), &[i], &[b], &[]).unwrap();
        assert_eq!(next["income_czk"], "100.00");
        assert_eq!(next["expenses_czk"], "60.00");
        assert_eq!(next["outstanding"].as_array().unwrap().len(), 0);
    }
    #[test]
    fn three_partial_payer_receipts_reconcile_to_net_invoice() {
        let i = invoice("121.00", "100.00", "CZK", "payer");
        let b = vec![
            bank("b1", "2026-01-01", "40.33", "CZK", true),
            bank("b2", "2026-02-01", "40.33", "CZK", true),
            bank("b3", "2026-03-01", "40.34", "CZK", true),
        ];
        let s = calculate(2026, &config(2026, "percentage"), &[i], &b, &[]).unwrap();
        assert_eq!(s["income_czk"], "100.00");
        let row_sum: Decimal = s["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["kind"] == "income")
            .map(|r| dec(r["taxable_czk"].as_str().unwrap()).unwrap())
            .sum();
        assert_eq!(row_sum, dec("100.00").unwrap());
    }
    #[test]
    fn payer_cent_allocation_carries_across_years() {
        let i = invoice("0.04", "0.03", "CZK", "payer");
        let b = vec![
            bank("b1", "2025-12-30", "0.02", "CZK", true),
            bank("b2", "2026-01-02", "0.02", "CZK", true),
        ];
        let prior = calculate(2025, &config(2025, "percentage"), &[i.clone()], &b, &[]).unwrap();
        let next = calculate(2026, &config(2026, "percentage"), &[i], &b, &[]).unwrap();
        assert_eq!(prior["income_czk"], "0.02");
        assert_eq!(next["income_czk"], "0.01");
        assert_eq!(dec(prior["income_czk"].as_str().unwrap()).unwrap() + dec(next["income_czk"].as_str().unwrap()).unwrap(), dec("0.03").unwrap());
        assert!(!codes(&next).contains(&"prior_payer_receipt_unresolved"));
    }
    #[test]
    fn prior_payer_override_blocks_current_automatic_income() {
        let i = invoice("0.04", "0.03", "CZK", "payer");
        let b = vec![bank("b1", "2025-12-30", "0.02", "CZK", true), bank("b2", "2026-01-02", "0.02", "CZK", true)];
        let mut overrides = BTreeSet::new();
        overrides.insert("inv1".to_string());
        let next = calculate_inner(2026, &config(2026, "percentage"), &[i], &b, &[], &overrides).unwrap();
        assert!(codes(&next).contains(&"prior_payer_receipt_unresolved"));
        assert_eq!(next["income_czk"], "0");
    }
    #[test]
    fn mixed_rates_split_one_receipt_without_double_counting_invoice() {
        let i = invoice("100", "100", "CZK", "nonpayer");
        let b = bank("b1", "2025-12-21", "100", "CZK", true);
        let mut c = config(2025, "percentage");
        c.bank_allocations = vec![
            BankAllocation {
                bank_id: "b1".into(),
                kind: "income".into(),
                category: "trade_80".into(),
                amount: "40".into(),
                ..Default::default()
            },
            BankAllocation {
                bank_id: "b1".into(),
                kind: "income".into(),
                category: "trade_60".into(),
                amount: "60".into(),
                ..Default::default()
            },
        ];
        let s = calculate(2025, &c, &[i], &[b], &[]).unwrap();
        assert_eq!(s["income_czk"], "100");
        assert_eq!(s["expenses_czk"], "68");
        assert_eq!(
            s["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "income")
                .count(),
            2
        );
    }
    #[test]
    fn unbalanced_split_is_excluded_from_provisional_totals() {
        let mut c = config(2025, "percentage");
        c.bank_allocations = vec![BankAllocation {
            bank_id: "b1".into(),
            kind: "income".into(),
            category: "trade_60".into(),
            amount: "120".into(),
            taxable_czk: "120".into(),
            note: "source".into(),
            ..Default::default()
        }];
        let s = calculate(
            2025,
            &c,
            &[],
            &[bank("b1", "2025-02-01", "100", "CZK", false)],
            &[],
        )
        .unwrap();
        assert_eq!(s["income_czk"], "0");
        assert!(codes(&s).contains(&"bank_allocation_mismatch"));
    }
    #[test]
    fn missing_category_and_fx_are_visible_blockers() {
        let i = invoice("100", "100", "EUR", "nonpayer");
        let b = bank("b1", "2025-12-21", "100", "EUR", true);
        let mut c = config(2025, "percentage");
        c.default_category = None;
        let s = calculate(2025, &c, &[i], &[b], &[]).unwrap();
        assert!(codes(&s).contains(&"income_category_missing"));
        assert!(codes(&s).contains(&"fx_conversion_missing"));
        assert_eq!(s["income_czk"], "0");
        assert_eq!(s["complete"], false);
    }
    #[test]
    fn percentage_cap_and_actual_expense_payment_date() {
        let mut c = config(2025, "percentage");
        c.manual_entries.push(ManualEntry {
            id: "cash1".into(),
            date: "2025-07-01".into(),
            kind: "income".into(),
            category: "trade_60".into(),
            amount: "2100000".into(),
            currency: "CZK".into(),
            taxable_czk: "2100000".into(),
            evidence: "cash ledger".into(),
            ..Default::default()
        });
        let s = calculate(2025, &c, &[], &[], &[]).unwrap();
        assert_eq!(s["expenses_czk"], "1200000");
        let p = Purchase {
            id: "p1".into(),
            supplier: "Vendor".into(),
            supplier_dic: "".into(),
            document_number: "P1".into(),
            supply_date: "2025-12-29".into(),
            currency: "CZK".into(),
            base: "100".into(),
            vat: "0".into(),
            vat_rate: "".into(),
            treatment: "".into(),
            note: "".into(),
        };
        let mut actual = config(2025, "actual");
        let unpaid = calculate(2025, &actual, &[], &[], &[p.clone()]).unwrap();
        assert_eq!(unpaid["expenses_czk"], "0");
        assert!(codes(&unpaid).contains(&"purchase_payment_unknown"));
        actual.manual_entries.push(ManualEntry {
            id: "cashexpense".into(),
            date: "2025-12-30".into(),
            kind: "expense".into(),
            amount: "100".into(),
            currency: "CZK".into(),
            taxable_czk: "100".into(),
            evidence: "cash book receipt".into(),
            purchase_id: "p1".into(),
            ..Default::default()
        });
        let paid = calculate(2025, &actual, &[], &[], &[p]).unwrap();
        assert_eq!(paid["expenses_czk"], "100");
        assert!(!codes(&paid).contains(&"purchase_payment_unknown"));
    }
    #[test]
    fn archive_requires_year_specific_reconciliation() {
        let mut i = invoice("100", "100", "CZK", "nonpayer");
        i.status = "archived".into();
        let s = calculate(2025, &config(2025, "actual"), &[i.clone()], &[], &[]).unwrap();
        assert!(codes(&s).contains(&"archive_payment_unknown"));
        assert_eq!(s["outstanding"].as_array().unwrap().len(), 0);
        let mut c = config(2025, "actual");
        c.reviewed_archive_ids.push("inv1".into());
        let reviewed = calculate(2025, &c, &[i], &[], &[]).unwrap();
        assert!(!codes(&reviewed).contains(&"archive_payment_unknown"));
    }
    #[test]
    fn overpayment_is_unresolved_and_excluded_from_income() {
        let i = invoice("100", "100", "CZK", "nonpayer");
        let b = bank("b1", "2025-12-21", "120", "CZK", true);
        let s = calculate(2025, &config(2025, "percentage"), &[i], &[b], &[]).unwrap();
        assert!(codes(&s).contains(&"invoice_overpaid"));
        assert_eq!(s["income_czk"], "0");
    }
    #[test]
    fn archived_receipt_needs_evidence_and_invalid_archive_total_blocks_completion() {
        let mut i = invoice("100", "100", "CZK", "nonpayer");
        i.status = "archived".into();
        i.regime = None;
        let mut c = config(2025, "actual");
        c.reviewed_archive_ids.push("inv1".into());
        c.bank_allocations.push(BankAllocation {
            bank_id: "b1".into(),
            kind: "income".into(),
            amount: "100".into(),
            taxable_czk: "100".into(),
            ..Default::default()
        });
        let b = bank("b1", "2025-12-21", "100", "CZK", true);
        let s = calculate(2025, &c, &[i.clone()], &[b], &[]).unwrap();
        assert!(codes(&s).contains(&"income_evidence_missing"));
        assert_eq!(s["income_czk"], "0");
        i.totals.gross = "broken".into();
        let s = calculate(2025, &c, &[i], &[], &[]).unwrap();
        assert!(codes(&s).contains(&"archive_amount_invalid"));
    }
    #[tokio::test]
    async fn csv_contains_status_and_confirmation_expires_after_record_change() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
            "CREATE TABLE invoices(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
            "CREATE TABLE bank_transactions(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
            "CREATE TABLE purchases(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        let mut c = serde_json::to_value(config(2025, "actual")).unwrap();
        c["confirm_records_complete"] = json!(true);
        annual_save_config(&pool, 2025, c).await.unwrap();
        let before = annual_summary(&pool, 2025).await.unwrap();
        assert_eq!(before["records_complete"], true);
        let csv = annual_csv(&pool, 2025).await.unwrap();
        assert!(csv.contains("PODKLAD §7"));
        assert!(csv.contains("Úplné,true"));
        let b = bank("b1", "2025-06-01", "100", "CZK", false);
        sqlx::query("INSERT INTO bank_transactions(id,data) VALUES(?,?)")
            .bind(&b.id)
            .bind(serde_json::to_string(&b).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        let after = annual_summary(&pool, 2025).await.unwrap();
        assert_eq!(after["records_complete"], false);
        assert!(codes(&after).contains(&"bank_unreviewed"));
        let csv = annual_csv(&pool, 2025).await.unwrap();
        assert!(csv.contains("Nedokončeno"));
        assert!(csv.contains("bank_unreviewed"));
    }
    #[tokio::test]
    async fn prior_year_classification_edit_expires_later_confirmation() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
            "CREATE TABLE invoices(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
            "CREATE TABLE bank_transactions(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
            "CREATE TABLE purchases(id TEXT PRIMARY KEY,data TEXT NOT NULL)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        annual_save_config(&pool, 2025, serde_json::to_value(config(2025, "percentage")).unwrap())
            .await.unwrap();
        let mut current = serde_json::to_value(config(2026, "actual")).unwrap();
        current["confirm_records_complete"] = json!(true);
        annual_save_config(&pool, 2026, current).await.unwrap();
        assert_eq!(annual_summary(&pool, 2026).await.unwrap()["records_complete"], true);

        let mut prior = annual_config(&pool, 2025).await.unwrap();
        prior["confirm_records_complete"] = json!(true);
        annual_save_config(&pool, 2025, prior).await.unwrap();
        assert_eq!(annual_summary(&pool, 2026).await.unwrap()["records_complete"], true);

        let mut changed = annual_config(&pool, 2025).await.unwrap();
        changed["default_category"] = json!("trade_80");
        annual_save_config(&pool, 2025, changed).await.unwrap();
        let later = annual_summary(&pool, 2026).await.unwrap();
        assert_eq!(later["records_complete"], false);
        assert!(codes(&later).contains(&"records_not_confirmed"));
    }
}
