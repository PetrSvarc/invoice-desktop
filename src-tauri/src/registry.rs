//! Read-only public business lookup. Results never change stored records on their own.
use crate::core::Result;
use chrono::Utc;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const ARES: &str = "https://ares.gov.cz/ekonomicke-subjekty-v-be/rest/ekonomicke-subjekty/";
const VAT: &str = "https://mojedane.gov.cz/dpr/axis2/services/rozhraniCRPDPH.rozhraniCRPDPHSOAP";
const VAT_DOCS: &str = "https://adisspr.mfcr.cz/pmd/dokumentace/webove-sluzby-spolehlivost-platcu";
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct Subject {
    pub source: String,
    pub source_url: String,
    pub checked_at: String,
    pub ico: Option<String>,
    pub dic: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub country: Option<String>,
    pub vat_status: Option<String>,
    pub reliability: Option<String>,
    pub legal_form: Option<String>,
    pub registry_updated_at: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, (Instant, Subject)>,
    last_request: Option<Instant>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn clean(s: &str) -> Option<String> {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).and_then(clean)
}
fn number(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| {
        x.as_str()
            .map(str::to_owned)
            .or_else(|| x.as_u64().map(|n| n.to_string()))
    })
}
fn join(parts: Vec<Option<String>>, separator: &str) -> Option<String> {
    clean(
        &parts
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(separator),
    )
}
fn normalize(kind: &str, input: &str) -> Result<String> {
    if input.len() > 64 {
        return Err("Zadaný identifikátor je příliš dlouhý.".into());
    }
    let value: String = input
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_uppercase();
    match kind {
        "ico"
            if !value.is_empty()
                && value.len() <= 8
                && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Ok(format!("{value:0>8}"))
        }
        "ico" => Err("IČO musí obsahovat nejvýše 8 číslic; úvodní nuly se doplní.".into()),
        "dic" => {
            let digits = value.strip_prefix("CZ").unwrap_or(&value);
            if (8..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()) {
                Ok(format!("CZ{digits}"))
            } else {
                Err("Zadejte české DIČ: CZ a 8 až 10 číslic. Zahraniční DIČ tento konektor zatím nepodporuje.".into())
            }
        }
        _ => Err("Zvolte hledání podle IČO nebo DIČ.".into()),
    }
}
fn client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Faktura/0.1 public-registry-lookup")
        .build()
        .map_err(|_| "Připojení k registru nelze inicializovat.".into())
}
async fn read(request: RequestBuilder, source: &str) -> Result<Vec<u8>> {
    let mut response = request.send().await.map_err(|_| {
        format!("{source} není dostupný. Zkontrolujte připojení a zkuste hledání později.")
    })?;
    if response.status().as_u16() == 404 {
        return Err(format!("{source}: subjekt nebyl nalezen."));
    }
    if !response.status().is_success() {
        return Err(format!(
            "{source}: služba vrátila HTTP {}. Zkuste hledání později.",
            response.status().as_u16()
        ));
    }
    if response.content_length().unwrap_or(0) > MAX_RESPONSE as u64 {
        return Err("Registr vrátil příliš velkou odpověď.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("{source}: odpověď se nepodařilo načíst."))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE {
            return Err("Registr vrátil příliš velkou odpověď.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn ares_address(v: &Value) -> Option<String> {
    if let Some(address) = string(v, "textovaAdresa") {
        return Some(address);
    }
    let orient = join(
        vec![
            number(v, "cisloOrientacni"),
            string(v, "cisloOrientacniPismeno"),
        ],
        "",
    );
    let house = join(vec![number(v, "cisloDomovni"), orient], "/");
    let street = join(
        vec![
            string(v, "nazevUlice").or_else(|| string(v, "nazevCastiObce")),
            house,
        ],
        " ",
    );
    let locality = join(
        vec![
            number(v, "psc").or_else(|| string(v, "pscTxt")),
            string(v, "nazevMestskeCastiObvodu").or_else(|| string(v, "nazevObce")),
        ],
        " ",
    );
    join(vec![street, locality], ", ")
}
fn parse_ares(bytes: &[u8], expected: &str) -> Result<Subject> {
    let v: Value =
        serde_json::from_slice(bytes).map_err(|_| "ARES vrátil neplatnou odpověď.".to_string())?;
    if string(&v, "ico").as_deref() != Some(expected) {
        return Err("ARES vrátil jiné IČO; údaje nebyly použity.".into());
    }
    let mut warnings = vec![];
    if string(&v, "datumZaniku").is_some() {
        warnings.push(
            "ARES eviduje datum zániku subjektu. Před použitím údajů zkontrolujte jeho stav."
                .into(),
        );
    }
    let address = ares_address(&v["sidlo"]);
    if address.is_none() {
        warnings.push("ARES neposkytl sídlo; doplňte adresu ručně.".into());
    }
    let dic = string(&v, "dic");
    if dic.is_none() {
        warnings.push(
            "ARES neposkytl DIČ. Samotné IČO ani chybějící DIČ neurčuje registraci k DPH.".into(),
        );
    }
    Ok(Subject {
        source: "ARES".into(),
        source_url: format!("https://ares.gov.cz/ekonomicke-subjekty?ico={expected}"),
        checked_at: Utc::now().to_rfc3339(),
        ico: Some(expected.into()),
        dic,
        name: string(&v, "obchodniJmeno"),
        address,
        country: string(&v["sidlo"], "kodStatu").filter(|c| c.len() == 2),
        vat_status: None,
        reliability: None,
        legal_form: string(&v, "pravniForma"),
        registry_updated_at: string(&v, "datumAktualizace"),
        warnings,
    })
}
async fn ares(client: &Client, ico: &str) -> Result<Subject> {
    parse_ares(
        &read(client.get(format!("{ARES}{ico}")), "ARES").await?,
        ico,
    )
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(rename = "Body")]
    body: SoapBody,
}
#[derive(Deserialize)]
struct SoapBody {
    #[serde(rename = "StatusNespolehlivySubjektRozsirenyResponse")]
    response: Option<VatResponse>,
}
#[derive(Deserialize)]
struct VatResponse {
    status: ServiceStatus,
    #[serde(rename = "statusSubjektu", default)]
    subjects: Vec<VatSubject>,
}
#[derive(Deserialize)]
struct ServiceStatus {
    #[serde(rename = "@statusCode")]
    code: String,
}
#[derive(Deserialize)]
struct VatSubject {
    #[serde(rename = "@dic")]
    dic: String,
    #[serde(rename = "@typSubjektu")]
    kind: String,
    #[serde(rename = "@nespolehlivyPlatce")]
    unreliable: String,
    #[serde(rename = "nazevSubjektu")]
    name: Option<String>,
    #[serde(rename = "adresa")]
    address: Option<VatAddress>,
}
#[derive(Deserialize)]
struct VatAddress {
    #[serde(rename = "uliceCislo")]
    street: Option<String>,
    #[serde(rename = "castObce")]
    district: Option<String>,
    #[serde(rename = "mesto")]
    city: Option<String>,
    psc: Option<String>,
    #[serde(rename = "stat")]
    country: Option<String>,
}
fn parse_vat(bytes: &[u8], expected: &str) -> Result<Subject> {
    let xml = std::str::from_utf8(bytes)
        .map_err(|_| "Registr DPH vrátil neplatné kódování.".to_string())?;
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return Err("Registr DPH vrátil nepodporovaný XML dokument.".into());
    }
    let envelope: Envelope = quick_xml::de::from_str(xml)
        .map_err(|_| "Registr DPH vrátil neplatnou odpověď.".to_string())?;
    let response = envelope
        .body
        .response
        .ok_or("Registr DPH vrátil chybu služby; zkuste hledání později.")?;
    if response.status.code != "0" {
        return Err(
            "Registr DPH je dočasně nedostupný nebo probíhá údržba. Zkuste hledání později.".into(),
        );
    }
    let mut subjects = response.subjects.into_iter();
    let s = subjects
        .next()
        .ok_or("Registr DPH nevrátil požadovaný subjekt.")?;
    if subjects.next().is_some() || format!("CZ{}", s.dic) != expected {
        return Err("Registr DPH vrátil jiné DIČ; údaje nebyly použity.".into());
    }
    let status = match s.kind.as_str() {
        "PLATCE_DPH" => "Plátce DPH",
        "IDENTIFIKOVANA_OSOBA" => "Identifikovaná osoba",
        "SKUPINA_DPH" => "Skupina DPH",
        "NESPOLEHLIVA_OSOBA" => "Nespolehlivá osoba",
        "NENALEZEN" => return Err(
            "DIČ nebylo v Registru DPH nalezeno. To samo o sobě neznamená, že subjekt neexistuje."
                .into(),
        ),
        _ => return Err("Registr DPH vrátil neznámý typ subjektu.".into()),
    };
    let reliability = match s.unreliable.as_str() {
        "ANO" => "Nespolehlivý subjekt",
        "NE" => "Není evidován jako nespolehlivý",
        _ => "Spolehlivost nebyla ověřena",
    };
    let mut warnings = vec![];
    if s.unreliable == "ANO" || s.kind == "NESPOLEHLIVA_OSOBA" {
        warnings.push("Registr DPH eviduje subjekt jako nespolehlivý.".into());
    }
    let (address, country) = if let Some(a) = s.address {
        let country = a.country.as_deref().and_then(clean);
        let code = if country.as_deref().map(|c| c.to_lowercase()) == Some("česká republika".into())
        {
            Some("CZ".into())
        } else {
            None
        };
        (
            join(
                vec![
                    a.street.and_then(|x| clean(&x)),
                    a.district.and_then(|x| clean(&x)),
                    join(
                        vec![
                            a.psc.and_then(|x| clean(&x)),
                            a.city.and_then(|x| clean(&x)),
                        ],
                        " ",
                    ),
                    country.filter(|_| code.is_none()),
                ],
                ", ",
            ),
            code,
        )
    } else {
        (None, None)
    };
    if address.is_none() {
        warnings.push("Registr DPH neposkytl adresu; doplňte ji ručně.".into());
    }
    Ok(Subject {
        source: "Registr DPH".into(),
        source_url: VAT_DOCS.into(),
        checked_at: Utc::now().to_rfc3339(),
        ico: None,
        dic: Some(expected.into()),
        name: s.name.and_then(|s| clean(&s)),
        address,
        country,
        vat_status: Some(status.into()),
        reliability: Some(reliability.into()),
        legal_form: None,
        registry_updated_at: None,
        warnings,
    })
}
async fn vat(client: &Client, dic: &str) -> Result<Subject> {
    let digits = dic
        .strip_prefix("CZ")
        .ok_or("Podporováno je pouze české DIČ.")?;
    // digits has already passed strict validation; no arbitrary XML can enter the request.
    let body = format!(
        r#"<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/" xmlns:roz="http://adis.mfcr.cz/rozhraniCRPDPH/"><soapenv:Body><roz:StatusNespolehlivySubjektRozsirenyV2Request><roz:dic>{digits}</roz:dic></roz:StatusNespolehlivySubjektRozsirenyV2Request></soapenv:Body></soapenv:Envelope>"#
    );
    let request = client
        .post(VAT)
        .header("Content-Type", "text/xml; charset=utf-8")
        .header(
            "SOAPAction",
            "http://adis.mfcr.cz/rozhraniCRPDPH/getStatusNespolehlivySubjektRozsirenyV2",
        )
        .body(body);
    parse_vat(&read(request, "Registr DPH").await?, dic)
}
fn merge(mut ares: Subject, vat: Subject) -> Subject {
    ares.source = "ARES + Registr DPH".into();
    ares.vat_status = vat.vat_status;
    ares.reliability = vat.reliability;
    ares.warnings.extend(vat.warnings);
    ares
}
async fn fetch(kind: &str, id: &str) -> Result<Subject> {
    let client = client()?;
    if kind == "ico" {
        let mut found = ares(&client, id).await?;
        if let Some(dic) = found.dic.as_deref().and_then(|d| normalize("dic", d).ok()) {
            match vat(&client, &dic).await {
                Ok(v) => found = merge(found, v),
                Err(e) => found.warnings.push(format!(
                    "Údaje ARES byly načteny. Stav DPH nebyl ověřen: {e}"
                )),
            }
        }
        return Ok(found);
    }
    let vat_result = vat(&client, id).await;
    // A Czech VAT identifier is not generally an IČO. For 8-digit IDs only,
    // enrich from ARES if ARES itself explicitly confirms the exact same DIČ.
    if id.len() == 10 {
        if let Ok(mut a) = ares(&client, &id[2..]).await {
            if a.dic.as_deref() == Some(id) {
                return Ok(match vat_result {
                    Ok(v) => merge(a, v),
                    Err(e) => {
                        a.warnings.push(format!(
                            "DIČ odpovídá záznamu ARES. Stav DPH nebyl ověřen: {e}"
                        ));
                        a
                    }
                });
            }
        }
    }
    let mut found = vat_result?;
    found.warnings.push(
        "Registr DPH neposkytuje IČO. Pokud jej potřebujete na faktuře, doplňte jej samostatně."
            .into(),
    );
    Ok(found)
}
pub async fn lookup(kind: &str, input: &str) -> Result<Subject> {
    let id = normalize(kind, input)?;
    let key = format!("{kind}:{id}");
    // One in-flight lookup per process; a short-lived bounded cache avoids repeat traffic.
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .await;
    if let Some((created, subject)) = cache.entries.get(&key) {
        if created.elapsed() < Duration::from_secs(600) {
            return Ok(subject.clone());
        }
    }
    if cache
        .last_request
        .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
    {
        return Err("Počkejte chvíli před dalším hledáním.".into());
    }
    cache.last_request = Some(Instant::now());
    let result = fetch(kind, &id).await?;
    if cache.entries.len() >= 64 {
        cache.entries.clear();
    }
    cache.entries.insert(key, (Instant::now(), result.clone()));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vat_xml(kind: &str, id: &str, code: &str) -> String {
        format!(
            r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"><s:Body><StatusNespolehlivySubjektRozsirenyResponse xmlns="http://adis.mfcr.cz/rozhraniCRPDPH/"><status statusCode="{code}"/><statusSubjektu dic="{id}" typSubjektu="{kind}" nespolehlivyPlatce="NE"><nazevSubjektu>Test &amp; partner</nazevSubjektu><adresa><uliceCislo>Ulice 1</uliceCislo><mesto>Praha</mesto><psc>11000</psc><stat>Česká republika</stat></adresa></statusSubjektu></StatusNespolehlivySubjektRozsirenyResponse></s:Body></s:Envelope>"#
        )
    }
    #[test]
    fn validates_identifiers_without_guessing_personal_ico() {
        assert_eq!(normalize("ico", " 123 456 ").unwrap(), "00123456");
        assert_eq!(normalize("dic", "cz 1234567890").unwrap(), "CZ1234567890");
        for input in [
            "CZ12<34>",
            "DE123456789",
            "123",
            "CZ12345678901",
            "CZ/12345678",
        ] {
            assert!(normalize("dic", input).is_err());
        }
    }
    #[test]
    fn ares_identity_address_and_missing_dic() {
        let bytes=br#"{"ico":"00123456","obchodniJmeno":"Test company","sidlo":{"kodStatu":"CZ","textovaAdresa":"Ulice 1, Praha"}}"#;
        let result = parse_ares(bytes, "00123456").unwrap();
        assert_eq!(result.address.as_deref(), Some("Ulice 1, Praha"));
        assert!(result.dic.is_none());
        assert!(result.vat_status.is_none());
        assert!(parse_ares(bytes, "11111111").is_err());
    }
    #[test]
    fn address_fallback_retains_house_suffix() {
        let v = serde_json::json!({"nazevUlice":"Ulice","cisloDomovni":12,"cisloOrientacni":3,"cisloOrientacniPismeno":"a","nazevObce":"Praha","psc":11000});
        assert_eq!(ares_address(&v).unwrap(), "Ulice 12/3a, 11000 Praha");
    }
    #[test]
    fn identified_person_is_not_a_payer_or_guessed_ico() {
        let s = parse_vat(
            vat_xml("IDENTIFIKOVANA_OSOBA", "1234567890", "0").as_bytes(),
            "CZ1234567890",
        )
        .unwrap();
        assert_eq!(s.vat_status.as_deref(), Some("Identifikovaná osoba"));
        assert_eq!(s.name.as_deref(), Some("Test & partner"));
        assert_eq!(s.country.as_deref(), Some("CZ"));
        assert!(s.ico.is_none());
    }
    #[test]
    fn rejects_mismatches_outages_faults_and_not_found() {
        assert!(parse_vat(
            vat_xml("PLATCE_DPH", "11111111", "0").as_bytes(),
            "CZ22222222"
        )
        .is_err());
        assert!(parse_vat(
            vat_xml("PLATCE_DPH", "11111111", "3").as_bytes(),
            "CZ11111111"
        )
        .unwrap_err()
        .contains("nedostupný"));
        assert!(parse_vat(
            vat_xml("NENALEZEN", "11111111", "0").as_bytes(),
            "CZ11111111"
        )
        .is_err());
        assert!(parse_vat(b"<Envelope><Body><Fault/></Body></Envelope>", "CZ11111111").is_err());
        assert!(parse_vat(b"<!DOCTYPE test><Envelope/>", "CZ11111111").is_err());
    }
    #[tokio::test]
    #[ignore = "Explicit opt-in: contacts official public ARES and Czech VAT APIs"]
    async fn live_public_registry_lookup() {
        let a = fetch("ico", "27074358").await.unwrap();
        assert_eq!(a.ico.as_deref(), Some("27074358"));
        assert!(a.name.is_some());
        assert_eq!(a.vat_status.as_deref(), Some("Plátce DPH"));
        let d = fetch("dic", "CZ27074358").await.unwrap();
        assert_eq!(d.ico, a.ico);
        assert_eq!(d.name, a.name);
        assert_eq!(d.vat_status, a.vat_status);
    }
}
