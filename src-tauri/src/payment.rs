use crate::core::{Invoice, Result};
use chrono::NaiveDate;
use qrcode::{Color, EcLevel, QrCode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::str::FromStr;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PaymentAccount {
    pub bank_account: String,
    pub iban: String,
    pub bic: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct InvoicePayment {
    pub account_name: String,
    pub bank_name: String,
    pub bank_account: String,
    pub iban: String,
    pub bic: String,
    pub variable_symbol: String,
    pub constant_symbol: String,
    pub specific_symbol: String,
    pub method: &'static str,
    pub amount: String,
    pub currency: String,
    pub due_date: String,
    pub qr_svg: Option<String>,
    pub qr_payload: Option<String>,
    pub qr_error: Option<String>,
}

pub fn validate_symbol(value: &str, max_len: usize, label: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    if value.len() > max_len || !value.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("{label}: zadejte nejvýše {max_len} číslic"));
    }
    Ok(())
}

pub fn number_symbol(number: &str) -> String {
    number.chars().filter(char::is_ascii_digit).collect()
}

pub fn effective_variable_symbol(invoice: &Invoice) -> String {
    if invoice.variable_symbol.is_empty() {
        number_symbol(invoice.number.as_deref().unwrap_or(""))
    } else {
        invoice.variable_symbol.clone()
    }
}

pub fn symbols_match(a: &str, b: &str) -> bool {
    if a.is_empty()
        || b.is_empty()
        || validate_symbol(a, 10, "Variabilní symbol").is_err()
        || validate_symbol(b, 10, "Variabilní symbol").is_err()
    {
        return false;
    }
    a.trim_start_matches('0') == b.trim_start_matches('0')
}

fn checksum_11(value: &str) -> bool {
    const WEIGHTS: [u32; 10] = [1, 2, 4, 8, 5, 10, 9, 7, 3, 6];
    value
        .bytes()
        .rev()
        .enumerate()
        .map(|(i, c)| (c - b'0') as u32 * WEIGHTS[i])
        .sum::<u32>()
        % 11
        == 0
}

fn domestic_parts(value: &str) -> Result<(String, String, String)> {
    let (account, bank) = value
        .split_once('/')
        .ok_or("Číslo účtu musí mít tvar [předčíslí-]číslo/kód banky")?;
    if bank.len() != 4 || !bank.bytes().all(|c| c.is_ascii_digit()) || bank == "0000" {
        return Err("Kód banky musí mít čtyři číslice a nesmí být 0000".into());
    }
    let (prefix, number) = account.split_once('-').unwrap_or(("", account));
    if prefix.len() > 6
        || number.is_empty()
        || number.len() > 10
        || !prefix.bytes().all(|c| c.is_ascii_digit())
        || !number.bytes().all(|c| c.is_ascii_digit())
        || number.bytes().all(|c| c == b'0')
    {
        return Err("Předčíslí nebo číslo účtu má neplatný formát".into());
    }
    if !checksum_11(prefix) || !checksum_11(number) {
        return Err("Kontrolní součet českého čísla účtu nesouhlasí".into());
    }
    Ok((bank.into(), prefix.into(), number.into()))
}

fn mod_97(s: &str) -> Result<u32> {
    let mut remainder = 0u32;
    for c in s.bytes() {
        if c.is_ascii_digit() {
            remainder = (remainder * 10 + (c - b'0') as u32) % 97;
        } else if c.is_ascii_uppercase() {
            let n = (c - b'A') as u32 + 10;
            remainder = (remainder * 100 + n) % 97;
        } else {
            return Err("IBAN obsahuje neplatné znaky".into());
        }
    }
    Ok(remainder)
}

fn check_iban(iban: &str) -> Result<()> {
    let bytes = iban.as_bytes();
    if bytes.len() < 15
        || bytes.len() > 34
        || !bytes[..2].iter().all(u8::is_ascii_uppercase)
        || !bytes[2..4].iter().all(u8::is_ascii_digit)
        || !bytes[4..].iter().all(u8::is_ascii_alphanumeric)
    {
        return Err("IBAN má neplatný formát".into());
    }
    let check_digits = &iban[2..4];
    if !(2..=98).contains(&check_digits.parse::<u8>().unwrap_or(0)) {
        return Err("Kontrolní číslice IBAN jsou neplatné".into());
    }
    if iban.starts_with("CZ") && (iban.len() != 24 || !bytes[4..].iter().all(u8::is_ascii_digit)) {
        return Err("Český IBAN musí mít 24 znaků a číselný účet".into());
    }
    if mod_97(&format!("{}{}", &iban[4..], &iban[..4]))? != 1 {
        return Err("Kontrolní součet IBAN nesouhlasí".into());
    }
    if iban.starts_with("CZ") {
        let prefix = &iban[8..14];
        let number = &iban[14..24];
        if &iban[4..8] == "0000"
            || number.bytes().all(|c| c == b'0')
            || !checksum_11(prefix)
            || !checksum_11(number)
        {
            return Err("Český IBAN obsahuje neplatný kód banky nebo číslo účtu".into());
        }
    }
    Ok(())
}

fn iban_from_domestic(bank: &str, prefix: &str, number: &str) -> Result<String> {
    let bban = format!("{bank}{prefix:0>6}{number:0>10}");
    let check = 98 - mod_97(&format!("{bban}CZ00"))?;
    Ok(format!("CZ{check:02}{bban}"))
}

fn domestic_from_iban(iban: &str) -> Result<String> {
    let bank = &iban[4..8];
    let prefix = &iban[8..14];
    let number = &iban[14..24];
    let prefix = prefix.trim_start_matches('0');
    let number = number.trim_start_matches('0');
    let account = if prefix.is_empty() {
        format!("{number}/{bank}")
    } else {
        format!("{prefix}-{number}/{bank}")
    };
    domestic_parts(&account)?;
    Ok(account)
}

pub fn normalize_account(bank_account: &str, iban: &str, bic: &str) -> Result<PaymentAccount> {
    let bank_account = bank_account.trim().replace(' ', "");
    let iban = iban
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    let bic = bic.trim().to_ascii_uppercase();
    if !bic.is_empty() {
        let b = bic.as_bytes();
        if ![8, 11].contains(&b.len())
            || !b[..6].iter().all(u8::is_ascii_uppercase)
            || !b[6..].iter().all(u8::is_ascii_alphanumeric)
        {
            return Err("BIC/SWIFT musí mít 8 nebo 11 platných znaků".into());
        }
    }
    let domestic = if bank_account.is_empty() {
        None
    } else {
        Some(domestic_parts(&bank_account)?)
    };
    if !iban.is_empty() {
        check_iban(&iban)?;
    }
    let derived_iban = domestic
        .as_ref()
        .map(|(bank, prefix, number)| iban_from_domestic(bank, prefix, number))
        .transpose()?;
    if let Some(expected) = &derived_iban {
        if !iban.is_empty() && iban != *expected {
            return Err("Číslo účtu a IBAN označují rozdílné účty".into());
        }
    }
    let bank_account = if bank_account.is_empty() && iban.starts_with("CZ") {
        domestic_from_iban(&iban)?
    } else if let Some((bank, prefix, number)) = domestic {
        let prefix = prefix.trim_start_matches('0');
        let number = number.trim_start_matches('0');
        if prefix.is_empty() {
            format!("{number}/{bank}")
        } else {
            format!("{prefix}-{number}/{bank}")
        }
    } else {
        bank_account
    };
    Ok(PaymentAccount {
        bank_account,
        iban: if iban.is_empty() {
            derived_iban.unwrap_or_default()
        } else {
            iban
        },
        bic,
    })
}

fn svg_for_payload(payload: &str) -> Result<String> {
    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M)
        .map_err(|e| format!("QR platbu se nepodařilo vytvořit: {e}"))?;
    let width = code.width();
    let canvas = width + 8;
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\" viewBox=\"0 0 {canvas} {canvas}\" role=\"img\" aria-label=\"QR platba\"><rect width=\"{canvas}\" height=\"{canvas}\" fill=\"white\"/><path fill=\"black\" d=\"");
    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == Color::Dark {
                svg.push_str(&format!("M{} {}h1v1h-1z", x + 4, y + 4));
            }
        }
    }
    svg.push_str("\"/></svg>");
    Ok(svg)
}

fn qr_content(
    invoice: &Invoice,
    account: &PaymentAccount,
    vs: &str,
    ks: &str,
    ss: &str,
) -> Result<(String, String)> {
    if invoice.currency != "CZK" {
        return Err("QR platba podporuje pouze CZK".into());
    }
    if account.iban.is_empty() {
        return Err("Pro QR platbu chybí číslo účtu".into());
    }
    if !account.iban.starts_with("CZ") {
        return Err("QR platba vyžaduje český bankovní účet".into());
    }
    validate_symbol(vs, 10, "Variabilní symbol")?;
    validate_symbol(ks, 4, "Konstantní symbol")?;
    validate_symbol(ss, 10, "Specifický symbol")?;
    if vs.is_empty() {
        return Err("Pro QR platbu chybí variabilní symbol".into());
    }
    let amount = Decimal::from_str(&invoice.totals.gross)
        .map_err(|_| "Celková částka faktury má neplatný formát".to_string())?;
    if amount <= Decimal::ZERO
        || amount > Decimal::new(999_999_999, 2)
        || amount != amount.round_dp(2)
    {
        return Err("QR platba vyžaduje částku od 0,01 do 9 999 999,99 Kč s nejvýše dvěma desetinnými místy".into());
    }
    let due_bytes = invoice.due_date.as_bytes();
    if due_bytes.len() != 10
        || due_bytes[4] != b'-'
        || due_bytes[7] != b'-'
        || !due_bytes[..4].iter().all(u8::is_ascii_digit)
        || !due_bytes[5..7].iter().all(u8::is_ascii_digit)
        || !due_bytes[8..].iter().all(u8::is_ascii_digit)
        || &invoice.due_date[..4] == "0000"
    {
        return Err("Datum splatnosti pro QR platbu musí mít tvar RRRR-MM-DD".into());
    }
    let due = NaiveDate::parse_from_str(&invoice.due_date, "%Y-%m-%d")
        .map_err(|_| "Datum splatnosti faktury má neplatný formát".to_string())?;
    let mut payload = format!("SPD*1.0*ACC:{}", account.iban);
    payload.push_str(&format!("*AM:{amount:.2}*CC:CZK*X-VS:{vs}"));
    if !ks.is_empty() {
        payload.push_str(&format!("*X-KS:{ks}"));
    }
    if !ss.is_empty() {
        payload.push_str(&format!("*X-SS:{ss}"));
    }
    payload.push_str(&format!("*DT:{}", due.format("%Y%m%d")));
    let svg = svg_for_payload(&payload)?;
    Ok((payload, svg))
}

pub fn payment_for_invoice(invoice: &Invoice) -> InvoicePayment {
    let seller = invoice.seller.as_ref();
    let raw_account = seller.map(|s| s.bank_account.as_str()).unwrap_or("");
    let raw_iban = seller.map(|s| s.iban.as_str()).unwrap_or("");
    let raw_bic = seller.map(|s| s.bic.as_str()).unwrap_or("");
    let vs = effective_variable_symbol(invoice);
    let ks = if invoice.constant_symbol.is_empty() {
        seller
            .map(|s| s.constant_symbol.clone())
            .unwrap_or_default()
    } else {
        invoice.constant_symbol.clone()
    };
    let mut result = InvoicePayment {
        account_name: seller
            .map(|s| {
                if s.account_name.trim().is_empty() {
                    s.name.clone()
                } else {
                    s.account_name.clone()
                }
            })
            .unwrap_or_default(),
        bank_name: seller.map(|s| s.bank_name.clone()).unwrap_or_default(),
        bank_account: raw_account.into(),
        iban: raw_iban.into(),
        bic: raw_bic.into(),
        variable_symbol: vs.clone(),
        constant_symbol: ks.clone(),
        specific_symbol: invoice.specific_symbol.clone(),
        method: "bank_transfer",
        amount: invoice.totals.gross.clone(),
        currency: invoice.currency.clone(),
        due_date: invoice.due_date.clone(),
        qr_svg: None,
        qr_payload: None,
        qr_error: None,
    };
    if invoice.status != "issued" {
        result.qr_error = Some("QR platbu lze vytvořit jen pro vystavenou fakturu".into());
        return result;
    }
    match normalize_account(raw_account, raw_iban, raw_bic) {
        Ok(account) => {
            result.bank_account = account.bank_account.clone();
            result.iban = account.iban.clone();
            result.bic = account.bic.clone();
            match qr_content(invoice, &account, &vs, &ks, &invoice.specific_symbol) {
                Ok((payload, svg)) => {
                    result.qr_payload = Some(payload);
                    result.qr_svg = Some(svg);
                }
                Err(message) => result.qr_error = Some(message),
            }
        }
        Err(message) => result.qr_error = Some(message),
    }
    result
}

pub async fn invoice_payment(pool: &SqlitePool, id: &str) -> Result<InvoicePayment> {
    let raw: String = sqlx::query_scalar("SELECT data FROM invoices WHERE id=?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|_| "Invoice not found".to_string())?;
    let invoice: Invoice = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(payment_for_invoice(&invoice))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Business, Customer, Line, Totals};

    fn sample_invoice() -> Invoice {
        let seller = Business {
            name: "Synthetic Studio".into(),
            ico: "12345678".into(),
            dic: "CZ12345678".into(),
            address: "Karlova 12, 110 00 Praha 1".into(),
            bank_account: "19-2000145399/0800".into(),
            iban: "CZ6508000000192000145399".into(),
            account_name: "Synthetic Studio".into(),
            bank_name: "Banka 0800".into(),
            bic: "GIBACZPX".into(),
            constant_symbol: "0308".into(),
            ..Business::default()
        };
        Invoice {
            id: "synthetic-invoice".into(),
            customer_id: "synthetic-customer".into(),
            number: Some("2026-0001".into()),
            status: "issued".into(),
            issue_date: "2026-10-01".into(),
            due_date: "2026-10-15".into(),
            supply_date: "2026-10-01".into(),
            currency: "CZK".into(),
            lines: vec![Line {
                description: "Syntetická grafická služba".into(),
                quantity: "1".into(),
                unit_price: "100.00".into(),
                treatment: "domestic_standard".into(),
                vat_rate: "21".into(),
            }],
            totals: Totals {
                base: "100.00".into(),
                vat: "21.00".into(),
                gross: "121.00".into(),
            },
            regime: Some("payer".into()),
            seller: Some(seller),
            customer: Some(Customer {
                id: "synthetic-customer".into(),
                name: "Ukázkový zákazník s.r.o.".into(),
                ico: "87654321".into(),
                dic: "CZ87654321".into(),
                address: "Testovací 5, 602 00 Brno".into(),
                email: "test@example.invalid".into(),
                country: "CZ".into(),
            }),
            issued_at: Some("2026-10-01T10:00:00Z".into()),
            recurrence_id: None,
            variable_symbol: "0001234567".into(),
            constant_symbol: "0308".into(),
            specific_symbol: "0007".into(),
        }
    }

    #[test]
    fn czech_account_and_iban_are_consistent() {
        let account = normalize_account("19-2000145399/0800", "", "gibaczpx").unwrap();
        assert_eq!(account.iban, "CZ6508000000192000145399");
        assert_eq!(account.bic, "GIBACZPX");
        assert_eq!(
            normalize_account("", &account.iban, "")
                .unwrap()
                .bank_account,
            "19-2000145399/0800"
        );
        assert_eq!(
            normalize_account(&account.bank_account, &account.iban, "")
                .unwrap()
                .iban,
            account.iban
        );
    }

    #[test]
    fn account_errors_do_not_choose_one_of_two_conflicting_accounts() {
        assert!(normalize_account("19-2000145399/0800", "CZ6508000000192000145398", "").is_err());
        assert!(
            normalize_account("19-2000145399/0800", "CZ6508000000192000145399", "GIBACZP").is_err()
        );
        assert!(normalize_account("20-2000145399/0800", "", "").is_err());
        assert!(normalize_account("19-2000145398/0800", "", "").is_err());
        assert!(normalize_account("", "CZ6508000000192000145398", "").is_err());
        let mismatch =
            normalize_account("19-2000145399/0800", "CZ6508000000192000145399", "").unwrap();
        let other = normalize_account("19-2000145399/0100", "", "").unwrap();
        assert_ne!(mismatch.iban, other.iban);
        assert!(normalize_account("19-2000145399/0800", &other.iban, "").is_err());
        assert!(normalize_account("19-2000145399/0000", "", "").is_err());
        let invalid_bank_iban = iban_from_domestic("0000", "19", "2000145399").unwrap();
        assert!(normalize_account("", &invalid_bank_iban, "").is_err());
        assert!(normalize_account("", "CZ9700000000192000145399", "").is_err());
        for bad in [
            "CZ0008000000192000145399",
            "CZ0108000000192000145399",
            "CZ9908000000192000145399",
        ] {
            assert!(normalize_account("", bad, "").is_err());
        }
    }

    #[test]
    fn payment_qr_preserves_symbols_amount_due_date_and_quiet_zone() {
        let payment = payment_for_invoice(&sample_invoice());
        let payload = payment.qr_payload.unwrap();
        assert_eq!(payload, "SPD*1.0*ACC:CZ6508000000192000145399*AM:121.00*CC:CZK*X-VS:0001234567*X-KS:0308*X-SS:0007*DT:20261015");
        assert_eq!(payment.qr_error, None);
        let svg = payment.qr_svg.unwrap();
        assert!(svg.contains("fill=\"white\""));
        assert!(svg.contains("viewBox=\"0 0 "));
        assert!(svg.contains("M4 4") || svg.contains("M4 5"));
        assert!(!payload.contains("+GIBACZPX"));
    }

    #[test]
    fn no_qr_for_invalid_or_unsupported_payment() {
        let mut invoice = sample_invoice();
        invoice.currency = "EUR".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.currency = "CZK".into();
        invoice.totals.gross = "0".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.totals.gross = "10000000.00".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.totals.gross = "1.001".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.totals.gross = "121.00".into();
        invoice.variable_symbol = "12345678901".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.variable_symbol = "123".into();
        invoice.due_date = "+10000-10-15".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
        invoice.due_date = "2026-10-15".into();
        invoice.seller.as_mut().unwrap().iban = "CZ6508000000192000145398".into();
        let payment = payment_for_invoice(&invoice);
        assert!(payment.qr_svg.is_none());
        assert_eq!(payment.iban, "CZ6508000000192000145398");
        assert!(payment.qr_error.is_some());
        invoice.status = "draft".into();
        assert!(payment_for_invoice(&invoice).qr_svg.is_none());
    }

    #[test]
    fn legacy_json_uses_immutable_number_and_seller_snapshot() {
        let mut value = serde_json::to_value(sample_invoice()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("variable_symbol");
        object.remove("constant_symbol");
        object.remove("specific_symbol");
        object
            .get_mut("seller")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("account_name");
        let invoice: Invoice = serde_json::from_value(value).unwrap();
        let payment = payment_for_invoice(&invoice);
        assert_eq!(payment.variable_symbol, "20260001");
        assert_eq!(payment.constant_symbol, "0308");
        assert_eq!(payment.account_name, "Synthetic Studio");
        assert!(symbols_match("000123", "123"));
        assert!(!symbols_match("", "0"));
    }

    #[test]
    #[ignore = "Writes a synthetic payment fixture only when PAYMENT_FIXTURE_PATH is set"]
    fn export_synthetic_payment_fixture() {
        let path = std::env::var("PAYMENT_FIXTURE_PATH").expect("Set PAYMENT_FIXTURE_PATH");
        let invoice = sample_invoice();
        let payment = payment_for_invoice(&invoice);
        assert!(payment.qr_svg.is_some());
        std::fs::write(
            std::path::Path::new(&path).with_extension("svg"),
            payment.qr_svg.as_ref().unwrap(),
        )
        .unwrap();
        let breakdown = serde_json::json!([{
            "treatment": "domestic_standard", "rate": "21 %", "base": "100.00", "vat": "21.00"
        }]);
        let fixture =
            serde_json::json!({"invoice": invoice, "breakdown": breakdown, "payment": payment});
        std::fs::write(path, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
    }
}
