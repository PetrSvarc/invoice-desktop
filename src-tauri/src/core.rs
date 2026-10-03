use chrono::{Datelike, Local, NaiveDate, Utc};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqlitePoolOptions, Row, Sqlite, SqlitePool, Transaction};
use std::{collections::BTreeMap, path::Path, str::FromStr};

pub type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn today() -> String {
    Local::now().date_naive().to_string()
}
fn date(s: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(err)
}
fn decimal(s: &str) -> Result<Decimal> {
    Decimal::from_str(s).map_err(err)
}
fn money(d: Decimal) -> String {
    d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .to_string()
}
fn uid(prefix: &str) -> String {
    format!(
        "{}-{}",
        prefix,
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    pub from: String,
    pub regime: String,
    #[serde(default)]
    pub dic: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Business {
    pub name: String,
    #[serde(default)]
    pub ico: String,
    #[serde(default)]
    pub dic: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub bank_account: String,
    #[serde(default)]
    pub iban: String,
    #[serde(default)]
    pub account_name: String,
    #[serde(default)]
    pub bank_name: String,
    #[serde(default)]
    pub bic: String,
    #[serde(default)]
    pub constant_symbol: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub registrations: Vec<Registration>,
}
impl Default for Business {
    fn default() -> Self {
        Self {
            name: "Moje podnikání".into(),
            ico: String::new(),
            dic: String::new(),
            address: String::new(),
            bank_account: String::new(),
            iban: String::new(),
            account_name: String::new(),
            bank_name: String::new(),
            bic: String::new(),
            constant_symbol: String::new(),
            email: String::new(),
            registrations: vec![Registration {
                from: "2020-01-01".into(),
                regime: "nonpayer".into(),
                dic: String::new(),
            }],
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Customer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub ico: String,
    #[serde(default)]
    pub dic: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub email: String,
    #[serde(default = "default_country")]
    pub country: String,
}
fn default_country() -> String {
    "CZ".into()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Line {
    pub description: String,
    pub quantity: String,
    pub unit_price: String,
    pub treatment: String,
    #[serde(default)]
    pub vat_rate: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Totals {
    pub base: String,
    pub vat: String,
    pub gross: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: String,
    pub customer_id: String,
    pub number: Option<String>,
    pub status: String,
    pub issue_date: String,
    pub due_date: String,
    pub supply_date: String,
    pub currency: String,
    pub lines: Vec<Line>,
    pub totals: Totals,
    pub regime: Option<String>,
    pub seller: Option<Business>,
    pub customer: Option<Customer>,
    pub issued_at: Option<String>,
    pub recurrence_id: Option<String>,
    #[serde(default)]
    pub variable_symbol: String,
    #[serde(default)]
    pub constant_symbol: String,
    #[serde(default)]
    pub specific_symbol: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Recurrence {
    pub id: String,
    pub name: String,
    pub customer_id: String,
    pub next_date: String,
    pub day: u32,
    pub interval_months: u32,
    pub due_days: i64,
    pub currency: String,
    pub lines: Vec<Line>,
    pub active: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Purchase {
    pub id: String,
    pub supplier: String,
    #[serde(default)]
    pub supplier_dic: String,
    pub document_number: String,
    pub supply_date: String,
    pub currency: String,
    pub base: String,
    pub vat: String,
    #[serde(default)]
    pub vat_rate: String,
    pub treatment: String,
    pub note: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct BankTx {
    pub id: String,
    pub source: String,
    pub external_id: String,
    pub booked: String,
    pub amount: String,
    pub currency: String,
    pub variable_symbol: String,
    pub description: String,
    pub invoice_id: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub business: Business,
    pub customers: Vec<Customer>,
    pub invoices: Vec<Invoice>,
    pub recurrences: Vec<Recurrence>,
    #[serde(default)]
    pub recurrence_runs: Vec<Value>,
    pub purchases: Vec<Purchase>,
    pub bank: Vec<BankTx>,
    #[serde(default)]
    pub payment_summaries: Vec<Value>,
    #[serde(default)]
    pub currency_totals: Vec<Value>,
    pub jobs: Vec<Value>,
    pub exceptions: Vec<Value>,
    #[serde(default)]
    pub meta: Vec<Value>,
    pub demo: bool,
    pub today: String,
}

pub async fn open(path: &Path) -> Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(err)?;
    }
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .map_err(err)?;
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await
        .map_err(err)?;
    sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&pool)
        .await
        .map_err(err)?;
    for sql in [
        "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS customers(id TEXT PRIMARY KEY, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS invoices(id TEXT PRIMARY KEY, number TEXT UNIQUE, status TEXT NOT NULL, issue_date TEXT NOT NULL, supply_date TEXT NOT NULL, customer_id TEXT NOT NULL, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS counters(year INTEGER PRIMARY KEY, next INTEGER NOT NULL)",
        "CREATE TABLE IF NOT EXISTS recurrences(id TEXT PRIMARY KEY, next_date TEXT NOT NULL, active INTEGER NOT NULL, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS recurrence_runs(template_id TEXT NOT NULL, period TEXT NOT NULL, invoice_id TEXT NOT NULL, PRIMARY KEY(template_id, period))",
        "CREATE TABLE IF NOT EXISTS purchases(id TEXT PRIMARY KEY, supply_date TEXT NOT NULL, data TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS bank_transactions(id TEXT PRIMARY KEY, source TEXT NOT NULL, external_id TEXT NOT NULL, invoice_id TEXT, data TEXT NOT NULL, UNIQUE(source,external_id))",
        "CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY, kind TEXT NOT NULL, outcome TEXT NOT NULL, created_at TEXT NOT NULL, detail TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS exceptions(id TEXT PRIMARY KEY, kind TEXT NOT NULL, message TEXT NOT NULL, created_at TEXT NOT NULL, resolved INTEGER NOT NULL DEFAULT 0)",
    ] { sqlx::query(sql).execute(&pool).await.map_err(err)?; }
    sqlx::query("INSERT OR IGNORE INTO meta(key,value) VALUES('business',?)")
        .bind(serde_json::to_string(&Business::default()).map_err(err)?)
        .execute(&pool)
        .await
        .map_err(err)?;
    sqlx::query("INSERT OR IGNORE INTO meta(key,value) VALUES('demo','false')")
        .execute(&pool)
        .await
        .map_err(err)?;
    Ok(pool)
}
pub async fn business(pool: &SqlitePool) -> Result<Business> {
    let s: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='business'")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    serde_json::from_str(&s).map_err(err)
}
pub fn regime_at(b: &Business, on: &str) -> Result<String> {
    date(on)?;
    let mut selected: Option<&Registration> = None;
    for r in &b.registrations {
        date(&r.from)?;
        if r.from.as_str() <= on
            && selected
                .map(|s| s.from.as_str() < r.from.as_str())
                .unwrap_or(true)
        {
            selected = Some(r);
        }
    }
    selected
        .map(|r| r.regime.clone())
        .ok_or_else(|| format!("No tax registration applies on {on}"))
}
pub fn calculate(lines: &[Line], regime: &str) -> Result<Totals> {
    if lines.is_empty() {
        return Err("Add at least one line".into());
    }
    let (mut base, mut vat) = (Decimal::ZERO, Decimal::ZERO);
    for l in lines {
        if l.description.trim().is_empty() {
            return Err("Line description is required".into());
        }
        let qty = decimal(&l.quantity)?;
        let price = decimal(&l.unit_price)?;
        if qty <= Decimal::ZERO || price < Decimal::ZERO {
            return Err("Quantity must be positive and price nonnegative".into());
        }
        let line_base =
            (qty * price).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        let rate = match (regime, l.treatment.as_str()) {
            ("nonpayer", "nonpayer") | ("identified", "nonpayer") => Decimal::ZERO,
            ("payer", "domestic_standard") => Decimal::new(21, 2),
            ("payer", "domestic_reduced") => Decimal::new(12, 2),
            ("payer", "exempt") => return Err("Exempt supply needs a legal basis and is not supported for issuance yet".into()),
            _ => return Err(format!("Treatment '{}' is unsupported under '{}' registration; review this invoice manually", l.treatment, regime)),
        };
        base += line_base;
        vat += (line_base * rate).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    }
    Ok(Totals {
        base: money(base),
        vat: money(vat),
        gross: money(base + vat),
    })
}
pub async fn invoice_breakdown(pool: &SqlitePool, id: &str) -> Result<Value> {
    let raw: String =
        sqlx::query_scalar("SELECT data FROM invoices WHERE id=? AND status='issued'")
            .bind(id)
            .fetch_one(pool)
            .await
            .map_err(|_| "Issued invoice not found".to_string())?;
    let inv: Invoice = serde_json::from_str(&raw).map_err(err)?;
    let regime = inv
        .regime
        .as_deref()
        .ok_or("Issued invoice has no tax regime")?;
    let mut groups: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
    for l in &inv.lines {
        let base = (decimal(&l.quantity)? * decimal(&l.unit_price)?)
            .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        let rate = match (regime, l.treatment.as_str()) {
            ("payer", "domestic_standard") => Decimal::new(21, 2),
            ("payer", "domestic_reduced") => Decimal::new(12, 2),
            (_, "nonpayer") => Decimal::ZERO,
            _ => return Err("Unsupported archived invoice treatment".into()),
        };
        let vat = (base * rate).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        let x = groups
            .entry(l.treatment.clone())
            .or_insert((Decimal::ZERO, Decimal::ZERO));
        x.0 += base;
        x.1 += vat;
    }
    Ok(Value::Array(groups.into_iter().map(|(t,(b,v))|json!({"treatment":t,"rate":if t=="domestic_standard"{"21 %"}else if t=="domestic_reduced"{"12 %"}else{"—"},"base":money(b),"vat":money(v)})).collect()))
}
async fn from_table<T: for<'de> Deserialize<'de>>(
    pool: &SqlitePool,
    table: &str,
) -> Result<Vec<T>> {
    let query = format!("SELECT data FROM {table} ORDER BY rowid DESC");
    let rows: Vec<String> = sqlx::query_scalar(&query)
        .fetch_all(pool)
        .await
        .map_err(err)?;
    rows.iter()
        .map(|s| serde_json::from_str(s).map_err(err))
        .collect()
}
pub async fn snapshot(pool: &SqlitePool) -> Result<Snapshot> {
    let demo: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='demo'")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let jobs = sqlx::query("SELECT id,kind,outcome,created_at,detail FROM jobs ORDER BY created_at DESC LIMIT 100").fetch_all(pool).await.map_err(err)?.into_iter().map(|r| json!({"id":r.get::<String,_>(0),"kind":r.get::<String,_>(1),"outcome":r.get::<String,_>(2),"created_at":r.get::<String,_>(3),"detail":r.get::<String,_>(4)})).collect();
    let exceptions = sqlx::query("SELECT id,kind,message,created_at FROM exceptions WHERE resolved=0 ORDER BY created_at DESC").fetch_all(pool).await.map_err(err)?.into_iter().map(|r| json!({"id":r.get::<String,_>(0),"kind":r.get::<String,_>(1),"message":r.get::<String,_>(2),"created_at":r.get::<String,_>(3)})).collect();
    let recurrence_runs=sqlx::query("SELECT template_id,period,invoice_id FROM recurrence_runs").fetch_all(pool).await.map_err(err)?.into_iter().map(|r|json!({"template_id":r.get::<String,_>(0),"period":r.get::<String,_>(1),"invoice_id":r.get::<String,_>(2)})).collect();
    let meta = sqlx::query(
        "SELECT key,value FROM meta WHERE key NOT IN ('business','demo','fio_last_call') AND key NOT LIKE 'idoklad:file:%' AND key NOT LIKE 'idoklad:invoice:%'",
    )
    .fetch_all(pool)
    .await
    .map_err(err)?
    .into_iter()
    .map(|r| json!({"key":r.get::<String,_>(0),"value":r.get::<String,_>(1)}))
    .collect();
    let invoices: Vec<Invoice> = from_table(pool, "invoices").await?;
    let bank: Vec<BankTx> = from_table(pool, "bank_transactions").await?;
    let mut paid: BTreeMap<String, Decimal> = BTreeMap::new();
    for t in &bank {
        if let Some(id) = &t.invoice_id {
            *paid.entry(id.clone()).or_default() += decimal(&t.amount)?;
        }
    }
    let mut currency_totals: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut payment_summaries = Vec::new();
    for i in &invoices {
        if i.status != "issued" {
            continue;
        }
        let amount = decimal(&i.totals.gross)?;
        let applied = paid.get(&i.id).copied().unwrap_or_default();
        let remaining = (amount - applied).max(Decimal::ZERO);
        *currency_totals.entry(i.currency.clone()).or_default() += remaining;
        payment_summaries.push(json!({"invoice_id":i.id,"paid":money(applied),"outstanding":money(remaining),"paid_full":applied>=amount}));
    }
    let currency_totals = currency_totals
        .into_iter()
        .map(|(currency, amount)| json!({"currency":currency,"outstanding":money(amount)}))
        .collect();
    Ok(Snapshot {
        business: business(pool).await?,
        customers: from_table(pool, "customers").await?,
        invoices,
        recurrences: from_table(pool, "recurrences").await?,
        recurrence_runs,
        purchases: from_table(pool, "purchases").await?,
        bank,
        payment_summaries,
        currency_totals,
        jobs,
        exceptions,
        meta,
        demo: demo == "true",
        today: today(),
    })
}
pub async fn save_business(pool: &SqlitePool, mut b: Business) -> Result<()> {
    if b.name.trim().is_empty() {
        return Err("Business name is required".into());
    }
    if b.registrations.is_empty() {
        return Err("Add a dated tax registration".into());
    }
    for r in &b.registrations {
        date(&r.from)?;
        if !["nonpayer", "identified", "payer"].contains(&r.regime.as_str()) {
            return Err("Unknown tax regime".into());
        }
    }
    let account = crate::payment::normalize_account(&b.bank_account, &b.iban, &b.bic)?;
    b.bank_account = account.bank_account;
    b.iban = account.iban;
    b.bic = account.bic;
    crate::payment::validate_symbol(&b.constant_symbol, 4, "Konstantní symbol")?;
    sqlx::query("UPDATE meta SET value=? WHERE key='business'")
        .bind(serde_json::to_string(&b).map_err(err)?)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn save_customer(pool: &SqlitePool, mut c: Customer) -> Result<Customer> {
    if c.name.trim().is_empty() {
        return Err("Customer name is required".into());
    }
    if c.id.is_empty() {
        c.id = uid("cus");
    }
    sqlx::query("INSERT INTO customers(id,data) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET data=excluded.data").bind(&c.id).bind(serde_json::to_string(&c).map_err(err)?).execute(pool).await.map_err(err)?;
    Ok(c)
}
pub async fn save_invoice(pool: &SqlitePool, mut inv: Invoice) -> Result<Invoice> {
    if inv.id.is_empty() {
        inv.id = uid("inv");
    }
    let existing: Option<String> = sqlx::query_scalar("SELECT status FROM invoices WHERE id=?")
        .bind(&inv.id)
        .fetch_optional(pool)
        .await
        .map_err(err)?;
    if existing.as_deref() == Some("issued") {
        return Err("Issued invoice content is immutable".into());
    }
    crate::payment::validate_symbol(&inv.variable_symbol, 10, "Variabilní symbol")?;
    crate::payment::validate_symbol(&inv.constant_symbol, 4, "Konstantní symbol")?;
    crate::payment::validate_symbol(&inv.specific_symbol, 10, "Specifický symbol")?;
    let b = business(pool).await?;
    let regime = regime_at(&b, &inv.supply_date)?;
    date(&inv.issue_date)?;
    date(&inv.due_date)?;
    if inv.currency.trim().is_empty() {
        return Err("Currency is required".into());
    }
    inv.totals = calculate(&inv.lines, &regime)?;
    inv.regime = Some(regime);
    inv.status = "draft".into();
    inv.number = None;
    inv.seller = None;
    inv.customer = None;
    inv.issued_at = None;
    sqlx::query("INSERT INTO invoices(id,number,status,issue_date,supply_date,customer_id,data) VALUES(?,NULL,'draft',?,?,?,?) ON CONFLICT(id) DO UPDATE SET issue_date=excluded.issue_date,supply_date=excluded.supply_date,customer_id=excluded.customer_id,data=excluded.data")
        .bind(&inv.id).bind(&inv.issue_date).bind(&inv.supply_date).bind(&inv.customer_id).bind(serde_json::to_string(&inv).map_err(err)?).execute(pool).await.map_err(err)?;
    Ok(inv)
}
async fn issue_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    inv: &mut Invoice,
    b: &Business,
) -> Result<()> {
    let regime = regime_at(b, &inv.supply_date)?;
    if regime == "payer"
        && inv.currency != "CZK"
        && inv
            .lines
            .iter()
            .any(|l| l.treatment.starts_with("domestic_"))
    {
        return Err("Domestic VAT invoices in foreign currency require a snapshotted CZK exchange rate and tax conversion; issue in CZK for now".into());
    }
    inv.totals = calculate(&inv.lines, &regime)?;
    let customer_data: String = sqlx::query_scalar("SELECT data FROM customers WHERE id=?")
        .bind(&inv.customer_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| "Select an existing customer".to_string())?;
    let customer: Customer = serde_json::from_str(&customer_data).map_err(err)?;
    if b.ico.is_empty() || b.address.is_empty() {
        return Err("Complete business ID and address before issuing".into());
    }
    if customer.address.trim().is_empty() {
        return Err("Complete customer address before issuing".into());
    }
    let mut seller = b.clone();
    if let Some(reg) = b
        .registrations
        .iter()
        .filter(|r| r.from <= inv.supply_date)
        .max_by(|a, b| a.from.cmp(&b.from))
    {
        seller.dic = if reg.dic.trim().is_empty() {
            b.dic.clone()
        } else {
            reg.dic.clone()
        };
    }
    if regime != "nonpayer" && seller.dic.trim().is_empty() {
        return Err("DIČ is required for identified or VAT payer invoices".into());
    }
    let year = date(&inv.issue_date)?.year();
    sqlx::query("INSERT OR IGNORE INTO counters(year,next) VALUES(?,1)")
        .bind(year)
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    let n: i64 = sqlx::query_scalar("SELECT next FROM counters WHERE year=?")
        .bind(year)
        .fetch_one(&mut **tx)
        .await
        .map_err(err)?;
    sqlx::query("UPDATE counters SET next=next+1 WHERE year=?")
        .bind(year)
        .execute(&mut **tx)
        .await
        .map_err(err)?;
    inv.number = Some(format!("{year}-{n:04}"));
    crate::payment::validate_symbol(&inv.variable_symbol, 10, "Variabilní symbol")?;
    crate::payment::validate_symbol(&inv.constant_symbol, 4, "Konstantní symbol")?;
    crate::payment::validate_symbol(&inv.specific_symbol, 10, "Specifický symbol")?;
    if inv.variable_symbol.is_empty() {
        inv.variable_symbol = crate::payment::number_symbol(inv.number.as_deref().unwrap_or(""));
    }
    if inv.constant_symbol.is_empty() {
        inv.constant_symbol = seller.constant_symbol.clone();
    }
    inv.status = "issued".into();
    inv.regime = Some(regime);
    inv.seller = Some(seller);
    inv.customer = Some(customer);
    inv.issued_at = Some(Utc::now().to_rfc3339());
    sqlx::query(
        "UPDATE invoices SET number=?,status='issued',data=? WHERE id=? AND status='draft'",
    )
    .bind(inv.number.as_deref())
    .bind(serde_json::to_string(inv).map_err(err)?)
    .bind(&inv.id)
    .execute(&mut **tx)
    .await
    .map_err(err)?;
    Ok(())
}
pub async fn issue_invoice(pool: &SqlitePool, id: &str) -> Result<Invoice> {
    let mut tx = pool.begin().await.map_err(err)?;
    let data: String =
        sqlx::query_scalar("SELECT data FROM invoices WHERE id=? AND status='draft'")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| "Draft invoice not found".to_string())?;
    let mut inv: Invoice = serde_json::from_str(&data).map_err(err)?;
    let b = business(pool).await?;
    issue_in_tx(&mut tx, &mut inv, &b).await?;
    tx.commit().await.map_err(err)?;
    Ok(inv)
}
pub async fn save_recurrence(pool: &SqlitePool, mut r: Recurrence) -> Result<Recurrence> {
    date(&r.next_date)?;
    if r.id.is_empty() {
        r.id = uid("rec");
    }
    if r.interval_months == 0 || r.interval_months > 12 || r.day > 31 || r.day == 0 {
        return Err("Invalid recurrence interval or day".into());
    }
    let b = business(pool).await?;
    calculate(&r.lines, &regime_at(&b, &r.next_date)?)?;
    sqlx::query("INSERT INTO recurrences(id,next_date,active,data) VALUES(?,?,?,?) ON CONFLICT(id) DO UPDATE SET next_date=excluded.next_date,active=excluded.active,data=excluded.data")
        .bind(&r.id).bind(&r.next_date).bind(r.active).bind(serde_json::to_string(&r).map_err(err)?).execute(pool).await.map_err(err)?;
    Ok(r)
}
fn next_occurrence(on: NaiveDate, day: u32, interval: u32) -> Result<NaiveDate> {
    let total = on.year() * 12 + on.month0() as i32 + interval as i32;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) as u32 + 1;
    let last = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or("Invalid recurrence year")?
    .pred_opt()
    .ok_or("Invalid recurrence date")?
    .day();
    NaiveDate::from_ymd_opt(year, month, day.min(last))
        .ok_or_else(|| "Invalid recurrence date".into())
}
async fn record_job(pool: &SqlitePool, kind: &str, outcome: &str, detail: &str) -> Result<()> {
    sqlx::query("INSERT INTO jobs(id,kind,outcome,created_at,detail) VALUES(?,?,?,?,?)")
        .bind(uid("job"))
        .bind(kind)
        .bind(outcome)
        .bind(Utc::now().to_rfc3339())
        .bind(detail)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
async fn exception(pool: &SqlitePool, kind: &str, message: &str) -> Result<()> {
    sqlx::query("INSERT INTO exceptions(id,kind,message,created_at) VALUES(?,?,?,?)")
        .bind(uid("exc"))
        .bind(kind)
        .bind(message)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn run_due(pool: &SqlitePool, on: &str) -> Result<usize> {
    date(on)?;
    let b = business(pool).await?;
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT data FROM recurrences WHERE active=1 AND next_date<=?")
            .bind(on)
            .fetch_all(pool)
            .await
            .map_err(err)?;
    let mut count = 0;
    for raw in rows {
        let mut r: Recurrence = serde_json::from_str(&raw).map_err(err)?;
        for _ in 0..120 {
            if r.next_date.as_str() > on {
                break;
            }
            let occ = date(&r.next_date)?;
            let period = occ.format("%Y-%m").to_string();
            let result:Result<bool>=async {
                let mut tx=pool.begin().await.map_err(err)?;
                let seen:Option<i64>=sqlx::query_scalar("SELECT 1 FROM recurrence_runs WHERE template_id=? AND period=?").bind(&r.id).bind(&period).fetch_optional(&mut *tx).await.map_err(err)?;
                if seen.is_some() { tx.rollback().await.map_err(err)?; return Ok(false); }
                let due=date(on)?.checked_add_signed(chrono::Duration::days(r.due_days)).ok_or("Invalid due date")?.to_string();
                let mut inv=Invoice{id:uid("inv"),customer_id:r.customer_id.clone(),number:None,status:"draft".into(),issue_date:on.into(),due_date:due,supply_date:r.next_date.clone(),currency:r.currency.clone(),lines:r.lines.clone(),totals:Totals{base:"0".into(),vat:"0".into(),gross:"0".into()},regime:None,seller:None,customer:None,issued_at:None,recurrence_id:Some(r.id.clone()),variable_symbol:String::new(),constant_symbol:String::new(),specific_symbol:String::new()};
                sqlx::query("INSERT INTO invoices(id,number,status,issue_date,supply_date,customer_id,data) VALUES(?,NULL,'draft',?,?,?,?)").bind(&inv.id).bind(&inv.issue_date).bind(&inv.supply_date).bind(&inv.customer_id).bind(serde_json::to_string(&inv).map_err(err)?).execute(&mut *tx).await.map_err(err)?;
                issue_in_tx(&mut tx,&mut inv,&b).await?;
                sqlx::query("INSERT INTO recurrence_runs(template_id,period,invoice_id) VALUES(?,?,?)").bind(&r.id).bind(&period).bind(&inv.id).execute(&mut *tx).await.map_err(err)?;
                tx.commit().await.map_err(err)?; Ok(true)
            }.await;
            match result {
                Ok(true) => count += 1,
                Ok(false) => {}
                Err(e) => {
                    exception(
                        pool,
                        "recurrence",
                        &format!("{} · {}: {}", r.name, period, e),
                    )
                    .await?;
                    record_job(pool, "recurrence", "needs_attention", &e).await?;
                    break;
                }
            }
            r.next_date = next_occurrence(occ, r.day, r.interval_months)?.to_string();
            sqlx::query("UPDATE recurrences SET next_date=?,data=? WHERE id=?")
                .bind(&r.next_date)
                .bind(serde_json::to_string(&r).map_err(err)?)
                .bind(&r.id)
                .execute(pool)
                .await
                .map_err(err)?;
        }
    }
    if count > 0 {
        record_job(
            pool,
            "recurrence",
            "completed",
            &format!("Issued {count} invoice(s)"),
        )
        .await?;
    }
    Ok(count)
}
pub async fn save_purchase(pool: &SqlitePool, mut p: Purchase) -> Result<Purchase> {
    date(&p.supply_date)?;
    decimal(&p.base)?;
    decimal(&p.vat)?;
    if p.id.is_empty() {
        p.id = uid("pur")
    }
    sqlx::query("INSERT INTO purchases(id,supply_date,data) VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET supply_date=excluded.supply_date,data=excluded.data").bind(&p.id).bind(&p.supply_date).bind(serde_json::to_string(&p).map_err(err)?).execute(pool).await.map_err(err)?;
    Ok(p)
}
pub async fn import_bank(pool: &SqlitePool, source: &str, content: &str) -> Result<usize> {
    if source.trim().is_empty() {
        return Err("Bank import source is required".into());
    }
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers = rdr.headers().map_err(err)?.clone();
    let mut tx = pool.begin().await.map_err(err)?;
    let mut count = 0;
    for rec in rdr.records() {
        let rec = rec.map_err(err)?;
        let get = |name: &str| -> String {
            headers
                .iter()
                .position(|h| h.eq_ignore_ascii_case(name))
                .and_then(|i| rec.get(i))
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let external_id = get("id");
        let booked = get("date");
        let amount = get("amount");
        if external_id.is_empty() {
            return Err("CSV needs an id column".into());
        }
        date(&booked)?;
        decimal(&amount)?;
        let currency = get("currency");
        if currency.len() != 3 || !currency.bytes().all(|c| c.is_ascii_uppercase()) {
            return Err("Bank currency must be a three-letter uppercase code".into());
        }
        let mut h = Sha256::new();
        h.update(source.as_bytes());
        h.update(external_id.as_bytes());
        let id = hex::encode(h.finalize());
        let b = BankTx {
            id,
            source: source.into(),
            external_id,
            booked,
            amount,
            currency,
            variable_symbol: get("variable_symbol"),
            description: get("description"),
            invoice_id: None,
        };
        let res=sqlx::query("INSERT OR IGNORE INTO bank_transactions(id,source,external_id,invoice_id,data) VALUES(?,?,?,NULL,?)").bind(&b.id).bind(&b.source).bind(&b.external_id).bind(serde_json::to_string(&b).map_err(err)?).execute(&mut *tx).await.map_err(err)?;
        count += res.rows_affected() as usize;
    }
    tx.commit().await.map_err(err)?;
    record_job(
        pool,
        "bank_import",
        "completed",
        &format!("Imported {count} transaction(s) from {source}"),
    )
    .await?;
    Ok(count)
}
pub async fn background_tick(pool: &SqlitePool) -> Result<()> {
    run_due(pool, &today()).await?;
    Ok(())
}
pub async fn seed_demo(pool: &SqlitePool) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let clients: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    if count > 0 || clients > 0 {
        return Err("Demo data can only be added to an empty workspace".into());
    }
    let b = Business {
        name: "Ukázkové studio s.r.o.".into(),
        ico: "12345678".into(),
        dic: "CZ12345678".into(),
        address: "Karlova 12, 110 00 Praha 1".into(),
        bank_account: "19-2000145399/0800".into(),
        iban: "CZ6508000000192000145399".into(),
        account_name: "Ukázkové studio s.r.o.".into(),
        bank_name: "Ukázková banka".into(),
        bic: String::new(),
        constant_symbol: String::new(),
        email: "studio@example.test".into(),
        registrations: vec![Registration {
            from: "2020-01-01".into(),
            regime: "payer".into(),
            dic: "CZ12345678".into(),
        }],
    };
    save_business(pool, b).await?;
    let c = save_customer(
        pool,
        Customer {
            id: String::new(),
            name: "Acme Design s.r.o.".into(),
            ico: "87654321".into(),
            dic: "CZ87654321".into(),
            address: "Náměstí 5, 602 00 Brno".into(),
            email: "hello@example.test".into(),
            country: "CZ".into(),
        },
    )
    .await?;
    let d = today();
    let due = (Local::now().date_naive() + chrono::Duration::days(14)).to_string();
    let inv = save_invoice(
        pool,
        Invoice {
            id: String::new(),
            customer_id: c.id.clone(),
            number: None,
            status: "draft".into(),
            issue_date: d.clone(),
            due_date: due,
            supply_date: d.clone(),
            currency: "CZK".into(),
            lines: vec![Line {
                description: "Grafické služby".into(),
                quantity: "1".into(),
                unit_price: "12500".into(),
                treatment: "domestic_standard".into(),
                vat_rate: "21".into(),
            }],
            totals: Totals {
                base: "0".into(),
                vat: "0".into(),
                gross: "0".into(),
            },
            regime: None,
            seller: None,
            customer: None,
            issued_at: None,
            recurrence_id: None,
            variable_symbol: String::new(),
            constant_symbol: String::new(),
            specific_symbol: String::new(),
        },
    )
    .await?;
    issue_invoice(pool, &inv.id).await?;
    save_recurrence(
        pool,
        Recurrence {
            id: String::new(),
            name: "Měsíční správa".into(),
            customer_id: c.id,
            next_date: next_occurrence(Local::now().date_naive(), 1, 1)?.to_string(),
            day: 1,
            interval_months: 1,
            due_days: 14,
            currency: "CZK".into(),
            lines: vec![Line {
                description: "Měsíční správa webu".into(),
                quantity: "1".into(),
                unit_price: "4500".into(),
                treatment: "domestic_standard".into(),
                vat_rate: "21".into(),
            }],
            active: true,
        },
    )
    .await?;
    sqlx::query("UPDATE meta SET value='true' WHERE key='demo'")
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn clear_demo(pool: &SqlitePool) -> Result<()> {
    let demo: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='demo'")
        .fetch_one(pool)
        .await
        .map_err(err)?;
    if demo != "true" {
        return Err("Workspace is not in demo mode".into());
    }
    let mut tx = pool.begin().await.map_err(err)?;
    for table in [
        "customers",
        "invoices",
        "counters",
        "recurrences",
        "recurrence_runs",
        "purchases",
        "bank_transactions",
        "jobs",
        "exceptions",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    sqlx::query("DELETE FROM meta")
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("INSERT INTO meta(key,value) VALUES('business',?),('demo','false')")
        .bind(serde_json::to_string(&Business::default()).map_err(err)?)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    tx.commit().await.map_err(err)?;
    Ok(())
}
pub async fn match_bank(pool: &SqlitePool) -> Result<usize> {
    let bank: Vec<BankTx> = from_table(pool, "bank_transactions").await?;
    let inv: Vec<Invoice> = from_table(pool, "invoices").await?;
    let mut count = 0;
    let mut paid: BTreeMap<String, Decimal> = BTreeMap::new();
    for t in &bank {
        if let Some(id) = &t.invoice_id {
            if let Ok(a) = decimal(&t.amount) {
                *paid.entry(id.clone()).or_default() += a;
            }
        }
    }
    for mut t in bank.into_iter().filter(|t| t.invoice_id.is_none()) {
        let candidates: Vec<&Invoice> = inv
            .iter()
            .filter(|i| {
                i.status == "issued"
                    && i.currency == t.currency
                    && crate::payment::symbols_match(
                        &crate::payment::effective_variable_symbol(i),
                        &t.variable_symbol,
                    )
                    && decimal(&i.totals.gross).ok() == decimal(&t.amount).ok()
                    && decimal(&t.amount)
                        .map(|x| x > Decimal::ZERO)
                        .unwrap_or(false)
                    && paid.get(&i.id).copied().unwrap_or_default() == Decimal::ZERO
            })
            .collect();
        if candidates.len() == 1 {
            t.invoice_id = Some(candidates[0].id.clone());
            sqlx::query("UPDATE bank_transactions SET invoice_id=?,data=? WHERE id=? AND invoice_id IS NULL").bind(&t.invoice_id).bind(serde_json::to_string(&t).map_err(err)?).bind(&t.id).execute(pool).await.map_err(err)?;
            paid.insert(candidates[0].id.clone(), decimal(&t.amount)?);
            count += 1;
        }
    }
    record_job(
        pool,
        "bank_match",
        "completed",
        &format!("Matched {count} payment(s)"),
    )
    .await?;
    Ok(count)
}
pub async fn link_bank(pool: &SqlitePool, bank_id: &str, invoice_id: &str) -> Result<()> {
    let raw: String = sqlx::query_scalar("SELECT data FROM bank_transactions WHERE id=?")
        .bind(bank_id)
        .fetch_one(pool)
        .await
        .map_err(err)?;
    let mut t: BankTx = serde_json::from_str(&raw).map_err(err)?;
    let raw_inv: Option<String> = sqlx::query_scalar(
        "SELECT data FROM invoices WHERE id=? AND status IN ('issued','archived')",
    )
    .bind(invoice_id)
    .fetch_optional(pool)
    .await
    .map_err(err)?;
    let i: Invoice = serde_json::from_str(&raw_inv.ok_or("Issued or archived invoice not found")?)
        .map_err(err)?;
    if i.currency != t.currency || decimal(&t.amount)? <= Decimal::ZERO {
        return Err("Payment must be positive and use invoice currency".into());
    }
    t.invoice_id = Some(invoice_id.into());
    sqlx::query("UPDATE bank_transactions SET invoice_id=?,data=? WHERE id=?")
        .bind(invoice_id)
        .bind(serde_json::to_string(&t).map_err(err)?)
        .bind(bank_id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
pub async fn dismiss_exception(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE exceptions SET resolved=1 WHERE id=?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(err)?;
    Ok(())
}
async fn from_tx<T: for<'de> Deserialize<'de>>(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<Vec<T>> {
    let raw: Vec<String> =
        sqlx::query_scalar(&format!("SELECT data FROM {table} ORDER BY rowid DESC"))
            .fetch_all(&mut **tx)
            .await
            .map_err(err)?;
    raw.iter()
        .map(|s| serde_json::from_str(s).map_err(err))
        .collect()
}
pub async fn export_json(pool: &SqlitePool) -> Result<String> {
    let mut tx = pool.begin().await.map_err(err)?;
    let b_raw: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='business'")
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
    let d_raw: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='demo'")
        .fetch_one(&mut *tx)
        .await
        .map_err(err)?;
    let business: Business = serde_json::from_str(&b_raw).map_err(err)?;
    let jobs=sqlx::query("SELECT id,kind,outcome,created_at,detail FROM jobs ORDER BY created_at").fetch_all(&mut *tx).await.map_err(err)?.into_iter().map(|r|json!({"id":r.get::<String,_>(0),"kind":r.get::<String,_>(1),"outcome":r.get::<String,_>(2),"created_at":r.get::<String,_>(3),"detail":r.get::<String,_>(4)})).collect();
    let exceptions=sqlx::query("SELECT id,kind,message,created_at,resolved FROM exceptions ORDER BY created_at").fetch_all(&mut *tx).await.map_err(err)?.into_iter().map(|r|json!({"id":r.get::<String,_>(0),"kind":r.get::<String,_>(1),"message":r.get::<String,_>(2),"created_at":r.get::<String,_>(3),"resolved":r.get::<i64,_>(4)})).collect();
    let meta = sqlx::query(
        "SELECT key,value FROM meta WHERE key NOT IN ('business','demo','fio_last_call')",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(err)?
    .into_iter()
    .map(|r| json!({"key":r.get::<String,_>(0),"value":r.get::<String,_>(1)}))
    .collect();
    let recurrence_runs=sqlx::query("SELECT template_id,period,invoice_id FROM recurrence_runs").fetch_all(&mut *tx).await.map_err(err)?.into_iter().map(|r|json!({"template_id":r.get::<String,_>(0),"period":r.get::<String,_>(1),"invoice_id":r.get::<String,_>(2)})).collect();
    let snapshot = Snapshot {
        business,
        customers: from_tx(&mut tx, "customers").await?,
        invoices: from_tx(&mut tx, "invoices").await?,
        recurrences: from_tx(&mut tx, "recurrences").await?,
        recurrence_runs,
        purchases: from_tx(&mut tx, "purchases").await?,
        bank: from_tx(&mut tx, "bank_transactions").await?,
        payment_summaries: vec![],
        currency_totals: vec![],
        jobs,
        exceptions,
        meta,
        demo: d_raw == "true",
        today: today(),
    };
    tx.commit().await.map_err(err)?;
    serde_json::to_string_pretty(&json!({"format":"faktura-backup-v1","snapshot":snapshot}))
        .map_err(err)
}
pub async fn restore_json(pool: &SqlitePool, raw: &str) -> Result<()> {
    let v: Value = serde_json::from_str(raw).map_err(err)?;
    if v.get("format").and_then(Value::as_str) != Some("faktura-backup-v1") {
        return Err("Unsupported backup format".into());
    }
    let s: Snapshot = serde_json::from_value(v.get("snapshot").cloned().ok_or("Missing snapshot")?)
        .map_err(err)?;
    let mut tx = pool.begin().await.map_err(err)?;
    for table in [
        "customers",
        "invoices",
        "counters",
        "recurrences",
        "recurrence_runs",
        "purchases",
        "bank_transactions",
        "jobs",
        "exceptions",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    sqlx::query("UPDATE meta SET value=? WHERE key='business'")
        .bind(serde_json::to_string(&s.business).map_err(err)?)
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("UPDATE meta SET value=? WHERE key='demo'")
        .bind(if s.demo { "true" } else { "false" })
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    sqlx::query("DELETE FROM meta WHERE key NOT IN ('business','demo')")
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    for m in s.meta {
        sqlx::query("INSERT INTO meta(key,value) VALUES(?,?)")
            .bind(m["key"].as_str())
            .bind(m["value"].as_str())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for c in s.customers {
        sqlx::query("INSERT INTO customers(id,data) VALUES(?,?)")
            .bind(&c.id)
            .bind(serde_json::to_string(&c).map_err(err)?)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for i in s.invoices {
        sqlx::query("INSERT INTO invoices(id,number,status,issue_date,supply_date,customer_id,data) VALUES(?,?,?,?,?,?,?)").bind(&i.id).bind(&i.number).bind(&i.status).bind(&i.issue_date).bind(&i.supply_date).bind(&i.customer_id).bind(serde_json::to_string(&i).map_err(err)?).execute(&mut *tx).await.map_err(err)?;
    }
    for r in s.recurrences {
        sqlx::query("INSERT INTO recurrences(id,next_date,active,data) VALUES(?,?,?,?)")
            .bind(&r.id)
            .bind(&r.next_date)
            .bind(r.active)
            .bind(serde_json::to_string(&r).map_err(err)?)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for r in s.recurrence_runs {
        sqlx::query("INSERT INTO recurrence_runs(template_id,period,invoice_id) VALUES(?,?,?)")
            .bind(r["template_id"].as_str())
            .bind(r["period"].as_str())
            .bind(r["invoice_id"].as_str())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for p in s.purchases {
        sqlx::query("INSERT INTO purchases(id,supply_date,data) VALUES(?,?,?)")
            .bind(&p.id)
            .bind(&p.supply_date)
            .bind(serde_json::to_string(&p).map_err(err)?)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for b in s.bank {
        sqlx::query("INSERT INTO bank_transactions(id,source,external_id,invoice_id,data) VALUES(?,?,?,?,?)").bind(&b.id).bind(&b.source).bind(&b.external_id).bind(&b.invoice_id).bind(serde_json::to_string(&b).map_err(err)?).execute(&mut *tx).await.map_err(err)?;
    }
    for j in s.jobs {
        sqlx::query("INSERT INTO jobs(id,kind,outcome,created_at,detail) VALUES(?,?,?,?,?)")
            .bind(j["id"].as_str())
            .bind(j["kind"].as_str())
            .bind(j["outcome"].as_str())
            .bind(j["created_at"].as_str())
            .bind(j["detail"].as_str())
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    for e in s.exceptions {
        sqlx::query(
            "INSERT INTO exceptions(id,kind,message,created_at,resolved) VALUES(?,?,?,?,?)",
        )
        .bind(e["id"].as_str())
        .bind(e["kind"].as_str())
        .bind(e["message"].as_str())
        .bind(e["created_at"].as_str())
        .bind(e["resolved"].as_i64().unwrap_or(0))
        .execute(&mut *tx)
        .await
        .map_err(err)?;
    }
    let numbers: Vec<String> =
        sqlx::query_scalar("SELECT number FROM invoices WHERE number IS NOT NULL")
            .fetch_all(&mut *tx)
            .await
            .map_err(err)?;
    let mut counters: BTreeMap<i64, i64> = BTreeMap::new();
    for number in numbers {
        let Some((year_text, sequence_text)) = number.split_once('-') else {
            continue;
        };
        if year_text.len() != 4
            || sequence_text.len() < 4
            || sequence_text.len() > 9
            || !year_text.bytes().all(|b| b.is_ascii_digit())
            || !sequence_text.bytes().all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let (Ok(year), Ok(sequence)) = (year_text.parse::<i64>(), sequence_text.parse::<i64>())
        else {
            continue;
        };
        if !(2000..=9999).contains(&year) || sequence < 1 || sequence > 999_999_999 {
            continue;
        }
        counters
            .entry(year)
            .and_modify(|next| *next = (*next).max(sequence + 1))
            .or_insert(sequence + 1);
    }
    for (year, next) in counters {
        sqlx::query("INSERT INTO counters(year,next) VALUES(?,?)")
            .bind(year)
            .bind(next)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
    }
    tx.commit().await.map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn line(t: &str) -> Line {
        Line {
            description: "Service".into(),
            quantity: "2".into(),
            unit_price: "100.50".into(),
            treatment: t.into(),
            vat_rate: String::new(),
        }
    }
    #[test]
    fn money_and_treatment() {
        let x = calculate(&[line("domestic_standard")], "payer").unwrap();
        assert_eq!(x.base, "201.00");
        assert_eq!(x.vat, "42.21");
        assert_eq!(x.gross, "243.21");
        assert!(calculate(&[line("domestic_standard")], "identified").is_err());
    }
    #[test]
    fn month_end() {
        let x = date("2026-01-31").unwrap();
        assert_eq!(next_occurrence(x, 31, 1).unwrap().to_string(), "2026-02-28");
        assert_eq!(
            next_occurrence(date("2026-02-28").unwrap(), 31, 1)
                .unwrap()
                .to_string(),
            "2026-03-31"
        );
    }
    #[test]
    fn registration_transition() {
        let b = Business {
            registrations: vec![
                Registration {
                    from: "2020-01-01".into(),
                    regime: "identified".into(),
                    dic: "".into(),
                },
                Registration {
                    from: "2026-11-01".into(),
                    regime: "payer".into(),
                    dic: "".into(),
                },
            ],
            ..Business::default()
        };
        assert_eq!(regime_at(&b, "2026-10-31").unwrap(), "identified");
        assert_eq!(regime_at(&b, "2026-11-01").unwrap(), "payer");
    }
    #[tokio::test]
    async fn restoring_imported_number_does_not_create_local_counter() {
        let path = std::env::temp_dir().join(format!("faktura-counter-test-{}.sqlite", uid("db")));
        let pool = open(&path).await.unwrap();
        seed_demo(&pool).await.unwrap();
        let mut backup: Value = serde_json::from_str(&export_json(&pool).await.unwrap()).unwrap();
        let imported = backup["snapshot"]["invoices"][0].as_object().unwrap().clone();
        let mut imported = Value::Object(imported);
        imported["id"] = json!("imported-custom-number");
        imported["number"] = json!("20220001");
        imported["status"] = json!("archived");
        imported["issue_date"] = json!("2022-01-01");
        backup["snapshot"]["invoices"].as_array_mut().unwrap().push(imported);
        restore_json(&pool, &backup.to_string()).await.unwrap();
        let counter: Option<i64> = sqlx::query_scalar("SELECT next FROM counters WHERE year=2022")
            .fetch_optional(&pool).await.unwrap();
        assert_eq!(counter, None);
    }
    #[tokio::test]
    async fn recurrence_is_idempotent_and_snapshot_is_frozen() {
        let path = std::env::temp_dir().join(format!("faktura-test-{}.sqlite", uid("db")));
        let pool = open(&path).await.unwrap();
        save_business(
            &pool,
            Business {
                name: "Test Seller".into(),
                ico: "12345678".into(),
                dic: "CZ12345678".into(),
                address: "Praha".into(),
                bank_account: "19-2000145399/0800".into(),
                iban: "".into(),
                account_name: String::new(),
                bank_name: String::new(),
                bic: String::new(),
                constant_symbol: "0308".into(),
                email: "".into(),
                registrations: vec![Registration {
                    from: "2020-01-01".into(),
                    regime: "payer".into(),
                    dic: "CZ12345678".into(),
                }],
            },
        )
        .await
        .unwrap();
        let c = save_customer(
            &pool,
            Customer {
                id: String::new(),
                name: "Test Customer".into(),
                ico: "".into(),
                dic: "".into(),
                address: "Brno".into(),
                email: "".into(),
                country: "CZ".into(),
            },
        )
        .await
        .unwrap();
        let r = Recurrence {
            id: String::new(),
            name: "Monthly".into(),
            customer_id: c.id,
            next_date: "2026-01-31".into(),
            day: 31,
            interval_months: 1,
            due_days: 14,
            currency: "CZK".into(),
            lines: vec![line("domestic_standard")],
            active: true,
        };
        save_recurrence(&pool, r).await.unwrap();
        assert_eq!(run_due(&pool, "2026-01-31").await.unwrap(), 1);
        assert_eq!(run_due(&pool, "2026-01-31").await.unwrap(), 0);
        let before = snapshot(&pool).await.unwrap();
        assert_eq!(before.invoices.len(), 1);
        assert_eq!(before.recurrences[0].next_date, "2026-02-28");
        let issued = before.invoices[0].clone();
        assert_eq!(issued.seller.as_ref().unwrap().name, "Test Seller");
        assert_eq!(issued.variable_symbol, "20260001");
        assert_eq!(issued.constant_symbol, "0308");
        let first_payment = crate::payment::invoice_payment(&pool, &issued.id)
            .await
            .unwrap();
        assert_eq!(first_payment.iban, "CZ6508000000192000145399");
        assert!(first_payment.qr_svg.is_some());
        let mut changed = business(&pool).await.unwrap();
        changed.name = "Changed".into();
        changed.bank_account = "19-2000145399/0100".into();
        changed.iban.clear();
        changed.constant_symbol = "0558".into();
        save_business(&pool, changed).await.unwrap();
        assert_eq!(
            snapshot(&pool).await.unwrap().invoices[0]
                .seller
                .as_ref()
                .unwrap()
                .name,
            "Test Seller"
        );
        let frozen_payment = crate::payment::invoice_payment(&pool, &issued.id)
            .await
            .unwrap();
        assert_eq!(frozen_payment.iban, first_payment.iban);
        assert_eq!(frozen_payment.constant_symbol, "0308");
        assert!(save_invoice(&pool, issued).await.is_err());
        let csv="id,date,amount,currency,variable_symbol,description\ntx1,2026-01-31,243.21,CZK,20260001,Paid\n";
        assert_eq!(import_bank(&pool, "test", csv).await.unwrap(), 1);
        assert_eq!(import_bank(&pool, "test", csv).await.unwrap(), 0);
        let invalid="id,date,amount,currency,variable_symbol,description\ntx2,2026-01-31,10,CZK,,Valid first row\ntx3,2026-01-31,10,,,Missing currency\n";
        assert!(import_bank(&pool, "test", invalid).await.is_err());
        assert_eq!(
            snapshot(&pool).await.unwrap().bank.len(),
            1,
            "CSV import must roll back all rows on validation failure"
        );
        assert_eq!(match_bank(&pool).await.unwrap(), 1);
        assert_eq!(match_bank(&pool).await.unwrap(), 0);
        let backup = export_json(&pool).await.unwrap();
        restore_json(&pool, &backup).await.unwrap();
        assert_eq!(snapshot(&pool).await.unwrap().recurrence_runs.len(), 1);
        assert_eq!(run_due(&pool, "2026-01-31").await.unwrap(), 0);
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn custom_variable_symbol_matches_bank_without_ambiguous_auto_link() {
        let path = std::env::temp_dir().join(format!("faktura-test-{}.sqlite", uid("vs")));
        let pool = open(&path).await.unwrap();
        seed_demo(&pool).await.unwrap();
        let template = snapshot(&pool).await.unwrap().invoices.remove(0);
        let mut drafts = Vec::new();
        for symbol in ["0000123", "0123", "0000456"] {
            let mut draft = template.clone();
            draft.id.clear();
            draft.number = None;
            draft.variable_symbol = symbol.into();
            draft.constant_symbol = "0558".into();
            draft.specific_symbol = "0007".into();
            let draft = save_invoice(&pool, draft).await.unwrap();
            drafts.push(issue_invoice(&pool, &draft.id).await.unwrap());
        }
        let selected = crate::payment::invoice_payment(&pool, &drafts[2].id)
            .await
            .unwrap();
        assert_eq!(selected.variable_symbol, "0000456");
        assert_eq!(selected.constant_symbol, "0558");
        assert_eq!(selected.specific_symbol, "0007");
        assert!(selected
            .qr_payload
            .unwrap()
            .contains("*X-VS:0000456*X-KS:0558*X-SS:0007*"));
        let csv = "id,date,amount,currency,variable_symbol,description\nambiguous,2026-10-01,15125.00,CZK,123,Two candidates\nunique,2026-10-01,15125.00,CZK,456,One candidate\n";
        assert_eq!(import_bank(&pool, "symbols-test", csv).await.unwrap(), 2);
        assert_eq!(match_bank(&pool).await.unwrap(), 1);
        let bank = snapshot(&pool).await.unwrap().bank;
        assert_eq!(
            bank.iter()
                .find(|t| t.external_id == "ambiguous")
                .unwrap()
                .invoice_id,
            None
        );
        assert_eq!(
            bank.iter()
                .find(|t| t.external_id == "unique")
                .unwrap()
                .invoice_id
                .as_deref(),
            Some(drafts[2].id.as_str())
        );
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
