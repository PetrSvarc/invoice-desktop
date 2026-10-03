//! Conservative import of iDoklad's issued-invoice export for POHODA.
//! The XML is an accounting transfer, not a payment ledger or a historical tax profile.
use crate::core::{self, Customer, Invoice, Line, Result, Totals};
use chrono::NaiveDate;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use rust_decimal::Decimal;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;

const MAX_BYTES: usize = 5 * 1024 * 1024;
const MAX_INVOICES: usize = 1000;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 32;
const DAT_NS: &str = "http://www.stormware.cz/schema/version_2/data.xsd";
const INV_NS: &str = "http://www.stormware.cz/schema/version_2/invoice.xsd";
const TYP_NS: &str = "http://www.stormware.cz/schema/version_2/type.xsd";

#[derive(Default, Clone, Debug, Serialize)]
struct Node {
    name: String,
    ns: String,
    attrs: BTreeMap<String, String>,
    text: String,
    children: Vec<Node>,
}
impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children
            .iter()
            .find(|n| n.name == name && n.ns == expected_ns(name))
    }
    fn at(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |n, key| n.child(key))
    }
    fn value(&self, path: &[&str]) -> String {
        self.at(path)
            .map(|n| n.text.trim().to_owned())
            .unwrap_or_default()
    }
    fn attr(&self, key: &str) -> String {
        self.attrs.get(key).cloned().unwrap_or_default()
    }
    fn has(&self, name: &str) -> bool {
        self.name == name || self.children.iter().any(|n| n.has(name))
    }
}
fn expected_ns(name: &str) -> &'static str {
    match name {
        "dataPack" | "dataPackItem" => DAT_NS,
        "numberRequested" | "address" | "company" | "name" | "street" | "city" | "zip" | "ico"
        | "dic" | "email" | "country" | "ids" | "unitPrice" | "price" | "priceVAT" | "priceSum" => {
            TYP_NS
        }
        _ => INV_NS,
    }
}
fn local(name: &[u8]) -> String {
    String::from_utf8_lossy(name.rsplit(|c| *c == b':').next().unwrap_or(name)).into_owned()
}
fn xml_node(
    e: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    ns: String,
) -> Result<Node> {
    let mut n = Node {
        name: local(e.name().as_ref()),
        ns,
        ..Node::default()
    };
    for a in e.attributes().with_checks(true) {
        let a = a.map_err(|e| format!("Invalid XML attribute: {e}"))?;
        n.attrs.insert(
            local(a.key.as_ref()),
            a.decode_and_unescape_value(decoder)
                .map_err(|e| e.to_string())?
                .into_owned(),
        );
    }
    Ok(n)
}
fn parse_xml(raw: &str) -> Result<Node> {
    if raw.len() > MAX_BYTES {
        return Err("XML exceeds the 5 MiB import limit".into());
    }
    if raw.trim().is_empty() {
        return Err("Select an issued-invoice XML file".into());
    }
    let mut reader = NsReader::from_str(raw);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    let mut nodes = 0usize;
    loop {
        let decoder = reader.decoder();
        let (resolved, event) = reader
            .read_resolved_event()
            .map_err(|e| format!("Invalid XML: {e}"))?;
        let namespace = match resolved {
            ResolveResult::Bound(ns) => String::from_utf8_lossy(ns.as_ref()).into_owned(),
            ResolveResult::Unbound => String::new(),
            ResolveResult::Unknown(_) => {
                return Err("XML contains an undeclared namespace prefix".into())
            }
        };
        match event {
            Event::Start(e) => {
                nodes += 1;
                if nodes > MAX_NODES || stack.len() >= MAX_DEPTH {
                    return Err("XML complexity limit exceeded".into());
                }
                if ![DAT_NS, INV_NS, TYP_NS].contains(&namespace.as_str()) {
                    return Err("XML contains an unsupported namespace".into());
                }
                stack.push(xml_node(&e, decoder, namespace)?);
            }
            Event::Empty(e) => {
                nodes += 1;
                if nodes > MAX_NODES || stack.len() >= MAX_DEPTH {
                    return Err("XML complexity limit exceeded".into());
                }
                if ![DAT_NS, INV_NS, TYP_NS].contains(&namespace.as_str()) {
                    return Err("XML contains an unsupported namespace".into());
                }
                let n = xml_node(&e, decoder, namespace)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                } else if root.replace(n).is_some() {
                    return Err("XML contains multiple root elements".into());
                }
            }
            Event::End(e) => {
                let n = stack.pop().ok_or("Unexpected XML closing tag")?;
                if n.name != local(e.name().as_ref()) || n.ns != namespace {
                    return Err("Mismatched XML closing tag".into());
                }
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(n);
                } else if root.replace(n).is_some() {
                    return Err("XML contains multiple root elements".into());
                }
            }
            Event::Text(t) => {
                let decoded = t.decode().map_err(|e| e.to_string())?;
                let value = quick_xml::escape::unescape(&decoded)
                    .map_err(|e| format!("Invalid XML entity: {e}"))?;
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err("Text outside XML root".into());
                }
            }
            Event::CData(t) => {
                if let Some(n) = stack.last_mut() {
                    n.text.push_str(&t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::DocType(_) => return Err("DOCTYPE and external entities are not allowed".into()),
            Event::Decl(_) | Event::Comment(_) => {}
            Event::PI(_) => return Err("XML processing instructions are not allowed".into()),
            Event::Eof => break,
            _ => return Err("Unsupported XML construct".into()),
        }
    }
    if !stack.is_empty() {
        return Err("Unclosed XML element".into());
    }
    let root = root.ok_or("Missing XML root")?;
    if root.name != "dataPack" || root.ns != DAT_NS || root.attr("version") != "2.0" {
        return Err(
            "Expected POHODA version 2 dataPack XML with data, invoice, and type namespaces".into(),
        );
    }
    Ok(root)
}

fn dec(s: &str, label: &str) -> Result<Decimal> {
    Decimal::from_str(s.trim()).map_err(|_| format!("Invalid {label} amount '{s}'"))
}
fn date(s: &str, label: &str) -> Result<String> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map(|v| v.to_string())
        .map_err(|_| format!("Missing or invalid {label} date"))
}
fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn money(value: Decimal) -> String {
    value.normalize().to_string()
}

#[derive(Serialize)]
pub struct PreviewRow {
    pub source_id: String,
    pub number: String,
    pub issue_date: String,
    pub customer: String,
    pub gross: String,
    pub currency: String,
    pub status: String,
    pub reason: Option<String>,
}
#[derive(Serialize)]
pub struct Preview {
    pub sha256: String,
    pub source_sha256: Option<String>,
    pub source_format: &'static str,
    pub total: usize,
    pub importable: usize,
    pub duplicates: usize,
    pub blocked: usize,
    pub can_commit: bool,
    pub invoices: Vec<PreviewRow>,
    pub warnings: Vec<String>,
}
#[derive(Serialize)]
pub struct CommitSummary {
    pub imported: usize,
    pub duplicates: usize,
}
#[derive(Serialize)]
pub struct Source {
    pub content: String,
    pub filename: String,
    pub source_sha256: Option<String>,
}

struct Candidate {
    invoice: Invoice,
    customer: Customer,
    source_id: String,
    fingerprint: String,
    line_money: Vec<(String, String)>,
}

fn candidate(item: &Node) -> Result<Candidate> {
    let source_id = item.attr("id");
    if source_id.trim().is_empty() {
        return Err("Missing dataPackItem source ID".into());
    }
    let invoice = item.child("invoice").ok_or("dataPackItem has no invoice")?;
    let h = invoice
        .child("invoiceHeader")
        .ok_or("Missing invoiceHeader")?;
    if h.value(&["invoiceType"]) != "issuedInvoice" {
        return Err("Only issuedInvoice is supported".into());
    }
    for unsupported in [
        "foreignCurrency",
        "invoiceAdvancePaymentItem",
        "discountPercentage",
        "discount",
        "rounding",
        "typeCalculateVATInclusivePrice",
        "invoiceCancellation",
    ] {
        if invoice.has(unsupported) {
            return Err(format!("{unsupported} requires manual migration"));
        }
    }
    for unsupported in ["invoiceAdvancePaymentItem", "invoiceItemPayment"] {
        if invoice.children.iter().any(|n| n.name == unsupported)
            || invoice
                .child("invoiceDetail")
                .is_some_and(|d| d.children.iter().any(|n| n.name == unsupported))
        {
            return Err(format!("{unsupported} requires manual migration"));
        }
    }
    let number = [
        h.value(&["number", "numberRequested"]),
        h.value(&["number", "number"]),
        h.value(&["number", "numberReceived"]),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .ok_or("Missing original invoice number")?;
    let issue_date = date(&h.value(&["date"]), "issue")?;
    let due_date = date(&h.value(&["dateDue"]), "due")?;
    let supply_date = date(&h.value(&["dateTax"]), "tax supply")?;
    let currency = h.value(&["foreignCurrency", "currency", "ids"]);
    if !currency.is_empty() && currency != "CZK" {
        return Err(
            "Foreign-currency invoice needs its original currency amounts and rate; unsupported"
                .into(),
        );
    }
    let address = h
        .at(&["partnerIdentity", "address"])
        .ok_or("Missing customer address snapshot")?;
    let customer_name = [address.value(&["company"]), address.value(&["name"])]
        .into_iter()
        .find(|s| !s.is_empty())
        .ok_or("Missing customer name")?;
    let customer = Customer {
        id: format!("idoklad-cus-{}", digest(&number)),
        name: customer_name,
        ico: address.value(&["ico"]),
        dic: address.value(&["dic"]),
        address: [
            address.value(&["street"]),
            address.value(&["city"]),
            address.value(&["zip"]),
        ]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(", "),
        email: address.value(&["email"]),
        country: address.value(&["country", "ids"]),
    };
    let detail = invoice
        .child("invoiceDetail")
        .ok_or("Missing invoiceDetail")?;
    if detail.children.iter().any(|n| n.name != "invoiceItem") {
        return Err("Unsupported invoice detail item; manual migration required".into());
    }
    if detail.children.is_empty() {
        return Err("Invoice has no line items".into());
    }
    let mut lines = Vec::new();
    let mut line_money = Vec::new();
    let (mut base, mut vat) = (Decimal::ZERO, Decimal::ZERO);
    for (index, row) in detail.children.iter().enumerate() {
        let description = row.value(&["text"]);
        if description.is_empty() {
            return Err(format!("Line {} has no description", index + 1));
        }
        let quantity = if row.value(&["quantity"]).is_empty() {
            "1".to_owned()
        } else {
            row.value(&["quantity"])
        };
        let qty = dec(&quantity, "quantity")?;
        if qty <= Decimal::ZERO {
            return Err(format!("Line {} has non-positive quantity", index + 1));
        }
        let home = row
            .child("homeCurrency")
            .ok_or_else(|| format!("Line {} has no homeCurrency values", index + 1))?;
        let unit_price = home.value(&["unitPrice"]);
        if unit_price.is_empty() {
            return Err(format!("Line {} has no unitPrice", index + 1));
        }
        dec(&unit_price, "unit price")?;
        let b = dec(&home.value(&["price"]), "line base")?;
        let v = dec(&home.value(&["priceVAT"]), "line VAT")?;
        if b < Decimal::ZERO || v < Decimal::ZERO {
            return Err("Negative/credit amounts need manual migration".into());
        }
        base += b;
        vat += v;
        let rate = row
            .child("rateVAT")
            .map(|r| r.attr("value"))
            .unwrap_or_default();
        lines.push(Line {
            description,
            quantity,
            unit_price,
            treatment: "imported_unclassified".into(),
            vat_rate: rate,
        });
        line_money.push((money(b), money(v)));
    }
    let summary = invoice
        .child("invoiceSummary")
        .and_then(|s| s.child("homeCurrency"));
    let (total_base, total_vat, total_gross) = if let Some(s) = summary {
        let gross = s.value(&["priceSum"]);
        if gross.is_empty() {
            (base, vat, base + vat)
        } else {
            let g = dec(&gross, "invoice gross")?;
            (base, vat, g)
        }
    } else {
        (base, vat, base + vat)
    };
    if total_base < Decimal::ZERO || total_vat < Decimal::ZERO || total_gross < Decimal::ZERO {
        return Err("Negative/credit amounts need manual migration".into());
    }
    if (total_base + total_vat - total_gross).abs() > Decimal::new(1, 2) {
        return Err("Invoice summary differs from line amounts by more than 0.01".into());
    }
    // dataPackItem/@id can be a transfer-package identifier, so invoice number is the stable key.
    let id = format!("idoklad-inv-{}", digest(&number));
    let inv = Invoice {
        id,
        customer_id: customer.id.clone(),
        number: Some(number),
        status: "archived".into(),
        issue_date,
        due_date,
        supply_date,
        currency: "CZK".into(),
        lines,
        totals: Totals {
            base: money(total_base),
            vat: money(total_vat),
            gross: money(total_gross),
        },
        regime: None,
        seller: None,
        customer: Some(customer.clone()),
        issued_at: None,
        recurrence_id: None,
        variable_symbol: h.value(&["symVar"]),
        constant_symbol: h.value(&["symConst"]),
        specific_symbol: h.value(&["symSpec"]),
    };
    let fingerprint = digest(&serde_json::to_string(invoice).map_err(|e| e.to_string())?);
    Ok(Candidate {
        invoice: inv,
        customer,
        source_id,
        fingerprint,
        line_money,
    })
}

fn candidates(
    content: &str,
) -> Result<(
    String,
    Vec<std::result::Result<Candidate, (String, String)>>,
)> {
    let root = parse_xml(content)?;
    let business_ico = root.attr("ico");
    if business_ico.trim().is_empty() {
        return Err("dataPack is missing the exporting business IČO".into());
    }
    if root.children.is_empty() {
        return Err("No dataPackItem invoices in XML".into());
    }
    if root.children.len() > MAX_INVOICES {
        return Err("XML exceeds the 1000-invoice import limit".into());
    }
    let mut out = Vec::new();
    for (index, item) in root.children.iter().enumerate() {
        if item.name != "dataPackItem" || item.ns != DAT_NS {
            return Err("Only dataPackItem children are allowed".into());
        }
        let id = item.attr("id");
        out.push(candidate(item).map_err(|e| {
            (
                if id.is_empty() {
                    format!("row {}", index + 1)
                } else {
                    id
                },
                e,
            )
        }));
    }
    Ok((business_ico, out))
}

async fn classify(
    tx: &mut Transaction<'_, Sqlite>,
    c: &Candidate,
) -> Result<(String, Option<String>)> {
    let existing = sqlx::query("SELECT id FROM invoices WHERE number=? OR id=?")
        .bind(c.invoice.number.as_deref())
        .bind(&c.invoice.id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    if existing.is_empty() {
        return Ok(("importable".into(), None));
    }
    if existing.len() != 1 {
        return Ok((
            "blocked".into(),
            Some("Source ID and invoice number collide with different records".into()),
        ));
    }
    let existing_id = existing[0].get::<String, _>(0);
    let key = format!("idoklad:invoice:{existing_id}");
    let saved: Option<String> = sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
        .bind(&key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let same = saved
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["fingerprint"].as_str().map(str::to_owned))
        .as_deref()
        == Some(c.fingerprint.as_str());
    if same {
        Ok(("duplicate".into(), None))
    } else {
        Ok((
            "blocked".into(),
            Some(
                "Invoice number exists with different or unverified contents; review manually"
                    .into(),
            ),
        ))
    }
}

fn validate_source_sha256(value: Option<&str>) -> Result<Option<String>> {
    match value {
        None => Ok(None),
        Some(v) if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) => {
            Ok(Some(v.to_ascii_lowercase()))
        }
        Some(_) => Err("Invalid original-file SHA-256".into()),
    }
}
async fn inspect(
    tx: &mut Transaction<'_, Sqlite>,
    content: &str,
    source_sha256: Option<&str>,
) -> Result<(Preview, Vec<Candidate>)> {
    let (source_ico, parsed) = candidates(content)?;
    let raw_business: String = sqlx::query_scalar("SELECT value FROM meta WHERE key='business'")
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    let active_business: core::Business =
        serde_json::from_str(&raw_business).map_err(|e| e.to_string())?;
    if active_business.ico.trim().is_empty() {
        return Err("Set your business IČO before importing historical invoices".into());
    }
    if active_business.ico.trim() != source_ico.trim() {
        return Err("Exporting business IČO differs from this workspace".into());
    }
    let mut preview = Preview { sha256:digest(content), source_sha256:validate_source_sha256(source_sha256)?, source_format:"iDoklad POHODA issued-invoice XML", total:parsed.len(), importable:0, duplicates:0, blocked:0, can_commit:false, invoices:Vec::new(), warnings:vec!["Payment history, tax regime, and historical seller identity are not contained reliably in this export. Imported invoices are archived with payment state unknown.".into()] };
    let mut valid = Vec::new();
    let mut seen_numbers = HashSet::new();
    let mut seen_ids = HashSet::new();
    for entry in parsed {
        match entry {
            Ok(c) => {
                let number = c.invoice.number.clone().unwrap_or_default();
                let (status, reason) = if !seen_numbers.insert(number.clone())
                    || !seen_ids.insert(c.source_id.clone())
                {
                    (
                        "blocked".into(),
                        Some("Duplicate invoice number or source ID inside XML".into()),
                    )
                } else {
                    classify(tx, &c).await?
                };
                match status.as_str() {
                    "importable" => preview.importable += 1,
                    "duplicate" => preview.duplicates += 1,
                    _ => preview.blocked += 1,
                }
                preview.invoices.push(PreviewRow {
                    source_id: c.source_id.clone(),
                    number,
                    issue_date: c.invoice.issue_date.clone(),
                    customer: c.customer.name.clone(),
                    gross: c.invoice.totals.gross.clone(),
                    currency: c.invoice.currency.clone(),
                    status,
                    reason,
                });
                valid.push(c);
            }
            Err((source_id, reason)) => {
                preview.blocked += 1;
                preview.invoices.push(PreviewRow {
                    source_id,
                    number: String::new(),
                    issue_date: String::new(),
                    customer: String::new(),
                    gross: String::new(),
                    currency: String::new(),
                    status: "blocked".into(),
                    reason: Some(reason),
                });
            }
        }
    }
    preview.can_commit = preview.blocked == 0 && preview.importable > 0;
    Ok((preview, valid))
}

pub async fn preview(
    pool: &SqlitePool,
    content: &str,
    source_sha256: Option<&str>,
) -> Result<Preview> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let (result, _) = inspect(&mut tx, content, source_sha256).await?;
    tx.rollback().await.map_err(|e| e.to_string())?;
    Ok(result)
}

pub async fn commit(
    pool: &SqlitePool,
    content: &str,
    expected_sha256: &str,
    source_sha256: Option<&str>,
) -> Result<CommitSummary> {
    if digest(content) != expected_sha256 {
        return Err("Selected XML changed since preview; preview it again".into());
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let (preview, candidates) = inspect(&mut tx, content, source_sha256).await?;
    if !preview.can_commit {
        return Err(
            "Import cannot be committed; review blocked rows or duplicates in preview".into(),
        );
    }
    let mut imported = 0usize;
    for (row, c) in preview.invoices.iter().zip(candidates.iter()) {
        if row.status != "importable" {
            continue;
        }
        sqlx::query("INSERT INTO customers(id,data) VALUES(?,?)")
            .bind(&c.customer.id)
            .bind(serde_json::to_string(&c.customer).map_err(|e| e.to_string())?)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO invoices(id,number,status,issue_date,supply_date,customer_id,data) VALUES(?,?,?,?,?,?,?)")
            .bind(&c.invoice.id).bind(c.invoice.number.as_deref()).bind("archived").bind(&c.invoice.issue_date).bind(&c.invoice.supply_date).bind(&c.customer.id).bind(serde_json::to_string(&c.invoice).map_err(|e| e.to_string())?).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        let key = format!("idoklad:invoice:{}", c.invoice.id);
        let audit = serde_json::json!({"source_id":c.source_id,"number":c.invoice.number,"fingerprint":c.fingerprint,"file_sha256":preview.sha256,"source_sha256":preview.source_sha256,"line_money":c.line_money});
        sqlx::query("INSERT INTO meta(key,value) VALUES(?,?)")
            .bind(key)
            .bind(audit.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        imported += 1;
    }
    sqlx::query("INSERT OR IGNORE INTO meta(key,value) VALUES(?,?)")
        .bind(format!("idoklad:file:{}", preview.sha256))
        .bind(content)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    // Prevent the built-in YYYY-NNNN series from colliding with imported numbers.
    for c in candidates
        .iter()
        .filter(|c| c.invoice.number.as_deref().is_some())
    {
        let number = c.invoice.number.as_deref().unwrap_or_default();
        if let Some((year, seq)) = number.split_once('-') {
            if year.len() == 4
                && seq.len() == 4
                && year.chars().all(|v| v.is_ascii_digit())
                && seq.chars().all(|v| v.is_ascii_digit())
            {
                let year: i64 = year
                    .parse()
                    .map_err(|e| format!("Invalid invoice year: {e}"))?;
                let next: i64 = seq
                    .parse::<i64>()
                    .map_err(|e| format!("Invalid invoice sequence: {e}"))?
                    + 1;
                sqlx::query("INSERT INTO counters(year,next) VALUES(?,?) ON CONFLICT(year) DO UPDATE SET next=MAX(next,excluded.next)")
                    .bind(year).bind(next).execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
        }
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(CommitSummary {
        imported,
        duplicates: preview.duplicates,
    })
}

pub async fn source(pool: &SqlitePool, invoice_id: &str) -> Result<Source> {
    if !invoice_id.starts_with("idoklad-inv-") {
        return Err("Not an imported iDoklad invoice".into());
    }
    let key = format!("idoklad:invoice:{invoice_id}");
    let raw: String = sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Invoice provenance not found")?;
    let audit: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let sha = audit["file_sha256"]
        .as_str()
        .ok_or("Archived file hash missing")?;
    let source_sha256 = audit["source_sha256"].as_str().map(str::to_owned);
    let key = format!("idoklad:file:{sha}");
    let content: String = sqlx::query_scalar("SELECT value FROM meta WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Archived XML source not found")?;
    // The stored XML is decoded Unicode. Export it as valid UTF-8 even if the input declared Windows-1250.
    let body = if content.starts_with("<?xml") {
        content
            .find("?>")
            .map(|p| &content[p + 2..])
            .unwrap_or(&content)
    } else {
        &content
    };
    Ok(Source {
        content: format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{body}"),
        filename: format!("idoklad-issued-invoices-{}.xml", &sha[..12]),
        source_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn path(label: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("invoice-idoklad-{label}-{n}.sqlite"))
    }
    async fn db(label: &str) -> SqlitePool {
        let p = core::open(&path(label)).await.unwrap();
        let mut b = core::business(&p).await.unwrap();
        b.ico = "12345678".into();
        core::save_business(&p, b).await.unwrap();
        p
    }
    fn fixture(number: &str, source_id: &str, base: &str, vat: &str, gross: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<dat:dataPack xmlns:dat="http://www.stormware.cz/schema/version_2/data.xsd" xmlns:inv="http://www.stormware.cz/schema/version_2/invoice.xsd" xmlns:typ="http://www.stormware.cz/schema/version_2/type.xsd" version="2.0" ico="12345678">
<dat:dataPackItem id="{source_id}" version="2.0"><inv:invoice version="2.0">
<inv:invoiceHeader><inv:invoiceType>issuedInvoice</inv:invoiceType><inv:number><typ:numberRequested>{number}</typ:numberRequested></inv:number><inv:date>2022-03-01</inv:date><inv:dateDue>2022-03-15</inv:dateDue><inv:dateTax>2022-03-01</inv:dateTax><inv:partnerIdentity><typ:address><typ:company>Žluťoučký s.r.o.</typ:company><typ:street>Hlavní 1</typ:street><typ:city>Praha</typ:city><typ:zip>10000</typ:zip><typ:ico>87654321</typ:ico></typ:address></inv:partnerIdentity></inv:invoiceHeader>
<inv:invoiceDetail><inv:invoiceItem><inv:text>Služba</inv:text><inv:quantity>1</inv:quantity><inv:rateVAT value="21">high</inv:rateVAT><inv:homeCurrency><typ:unitPrice>{base}</typ:unitPrice><typ:price>{base}</typ:price><typ:priceVAT>{vat}</typ:priceVAT></inv:homeCurrency></inv:invoiceItem></inv:invoiceDetail>
<inv:invoiceSummary><inv:homeCurrency><typ:priceSum>{gross}</typ:priceSum></inv:homeCurrency></inv:invoiceSummary>
</inv:invoice></dat:dataPackItem></dat:dataPack>"#
        )
    }
    #[tokio::test]
    async fn preview_commit_duplicate_and_backup_roundtrip() {
        let pool = db("roundtrip").await;
        let xml = fixture("2022-0007", "source-7", "100.00", "21.00", "121.00");
        let original_hash = digest("original bytes for test");
        let p = preview(&pool, &xml, Some(&original_hash)).await.unwrap();
        assert_eq!((p.importable, p.duplicates, p.blocked), (1, 0, 0));
        let r = commit(&pool, &xml, &p.sha256, Some(&original_hash))
            .await
            .unwrap();
        assert_eq!(r.imported, 1);
        let again = preview(&pool, &xml, Some(&original_hash)).await.unwrap();
        assert_eq!(again.duplicates, 1);
        let reexport = fixture("2022-0007", "new-package-id", "100.00", "21.00", "121.00");
        assert_eq!(preview(&pool, &reexport, None).await.unwrap().duplicates, 1);
        let inv: Invoice = serde_json::from_str(
            &sqlx::query_scalar::<_, String>("SELECT data FROM invoices")
                .fetch_one(&pool)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(inv.status, "archived");
        assert_eq!(inv.totals.gross, "121");
        assert!(inv.seller.is_none() && inv.regime.is_none());
        let original = source(&pool, &inv.id).await.unwrap();
        assert_eq!(
            original.source_sha256.as_deref(),
            Some(original_hash.as_str())
        );
        assert!(original.content.contains("Žluťoučký"));
        let next: i64 = sqlx::query_scalar("SELECT next FROM counters WHERE year=2022")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(next, 8);
        let backup = core::export_json(&pool).await.unwrap();
        let restored = db("restored").await;
        core::restore_json(&restored, &backup).await.unwrap();
        assert!(source(&restored, &inv.id)
            .await
            .unwrap()
            .content
            .contains("Žluťoučký"));
        assert_eq!(preview(&restored, &xml, None).await.unwrap().duplicates, 1);
    }
    #[tokio::test]
    async fn changed_or_colliding_number_blocks_without_mutation() {
        let pool = db("collision").await;
        let xml = fixture("2022-0007", "source-7", "100.00", "21.00", "121.00");
        let p = preview(&pool, &xml, None).await.unwrap();
        commit(&pool, &xml, &p.sha256, None).await.unwrap();
        let changed = fixture("2022-0007", "source-7", "200.00", "42.00", "242.00");
        assert_eq!(preview(&pool, &changed, None).await.unwrap().blocked, 1);
        let collision = fixture("2022-0007", "source-other", "200.00", "42.00", "242.00");
        assert_eq!(preview(&pool, &collision, None).await.unwrap().blocked, 1);
        assert!(commit(&pool, &changed, &digest(&changed), None)
            .await
            .is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn unsupported_row_prevents_entire_commit_and_xxe_is_rejected() {
        let pool = db("reject").await;
        let xml = fixture("2022-0007", "source-7", "100.00", "21.00", "121.00");
        let second = fixture("2022-0008", "source-8", "100.00", "21.00", "121.00")
            .replace("issuedInvoice", "issuedProformaInvoice");
        let second_item = second
            .split("<dat:dataPackItem")
            .nth(1)
            .unwrap()
            .split("</dat:dataPackItem>")
            .next()
            .unwrap();
        let mixed = xml.replace(
            "</dat:dataPack>",
            &format!("<dat:dataPackItem{second_item}</dat:dataPackItem></dat:dataPack>"),
        );
        let p = preview(&pool, &mixed, None).await.unwrap();
        assert_eq!((p.importable, p.blocked), (1, 1));
        assert!(commit(&pool, &mixed, &p.sha256, None).await.is_err());
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
        let xxe = xml.replacen(
            "<dat:dataPack",
            "<!DOCTYPE foo [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><dat:dataPack",
            1,
        );
        assert!(preview(&pool, &xxe, None).await.is_err());
        assert!(preview(
            &pool,
            &xml.replace("ico=\"12345678\"", "ico=\"99999999\""),
            None
        )
        .await
        .is_err());
        let foreign = xml.replace(
            "<inv:invoiceDetail>",
            "<inv:foreignCurrency/><inv:invoiceDetail>",
        );
        assert_eq!(preview(&pool, &foreign, None).await.unwrap().blocked, 1);
    }
    #[tokio::test]
    async fn namespaces_follow_uri_and_reject_spoofed_invoice() {
        let pool = db("namespaces").await;
        let xml = fixture("2022-0007", "source-7", "100.00", "21.00", "121.00");
        let renamed = xml
            .replace("dat:", "d:")
            .replace("xmlns:dat", "xmlns:d")
            .replace("inv:", "i:")
            .replace("xmlns:inv", "xmlns:i")
            .replace("typ:", "t:")
            .replace("xmlns:typ", "xmlns:t");
        assert_eq!(preview(&pool, &renamed, None).await.unwrap().importable, 1);
        let decoded_windows_1250 = xml.replace("encoding=\"UTF-8\"", "encoding=\"Windows-1250\"");
        assert_eq!(
            preview(&pool, &decoded_windows_1250, None)
                .await
                .unwrap()
                .importable,
            1
        );
        let spoofed = xml
            .replace(
                "<inv:invoice version=",
                "<evil:invoice xmlns:evil=\"urn:bad\" version=",
            )
            .replace("</inv:invoice>", "</evil:invoice>");
        assert!(preview(&pool, &spoofed, None).await.is_err());
    }
}
