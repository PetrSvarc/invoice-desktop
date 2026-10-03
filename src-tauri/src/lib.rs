mod annual;
mod core;
mod idoklad;
mod payment;
mod registry;
mod tax;
use core::*;
use serde_json::{json, Value};
use sqlx::SqlitePool;
use tauri::{Manager, State};
use tax::*;

struct Db(SqlitePool);
fn parse<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Result<T> {
    serde_json::from_value(
        v.get(key)
            .cloned()
            .ok_or_else(|| format!("Missing {key}"))?,
    )
    .map_err(|e| e.to_string())
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing {key}"))
}

#[tauri::command]
async fn dispatch(
    db: State<'_, Db>,
    window: tauri::WebviewWindow,
    action: String,
    payload: Value,
) -> Result<Value> {
    let pool = &db.0;
    match action.as_str() {
        "snapshot" => serde_json::to_value(snapshot(pool).await?).map_err(|e| e.to_string()),
        "business.save" => {
            save_business(pool, parse(&payload, "business")?).await?;
            Ok(json!({"ok":true}))
        }
        "customer.save" => {
            serde_json::to_value(save_customer(pool, parse(&payload, "customer")?).await?)
                .map_err(|e| e.to_string())
        }
        "invoice.save" => {
            serde_json::to_value(save_invoice(pool, parse(&payload, "invoice")?).await?)
                .map_err(|e| e.to_string())
        }
        "invoice.issue" => serde_json::to_value(issue_invoice(pool, field(&payload, "id")?).await?)
            .map_err(|e| e.to_string()),
        "invoice.payment" => {
            serde_json::to_value(payment::invoice_payment(pool, field(&payload, "id")?).await?)
                .map_err(|e| e.to_string())
        }
        "payment.account" => serde_json::to_value(payment::normalize_account(
            field(&payload, "bank_account")?,
            field(&payload, "iban")?,
            field(&payload, "bic")?,
        )?)
        .map_err(|e| e.to_string()),
        "invoice.breakdown" => invoice_breakdown(pool, field(&payload, "id")?).await,
        "invoice.print" => {
            window.print().map_err(|e| e.to_string())?;
            Ok(json!({"ok":true}))
        }
        "recurrence.save" => {
            serde_json::to_value(save_recurrence(pool, parse(&payload, "recurrence")?).await?)
                .map_err(|e| e.to_string())
        }
        "recurrence.run" => Ok(json!({"issued":run_due(pool,field(&payload,"on")?).await?})),
        "purchase.save" => {
            serde_json::to_value(save_purchase(pool, parse(&payload, "purchase")?).await?)
                .map_err(|e| e.to_string())
        }
        "bank.import" => Ok(
            json!({"imported":import_bank(pool,field(&payload,"source")?,field(&payload,"content")?).await?}),
        ),
        "idoklad.preview" => serde_json::to_value(
            idoklad::preview(
                pool,
                field(&payload, "content")?,
                payload.get("source_sha256").and_then(Value::as_str),
            )
            .await?,
        )
        .map_err(|e| e.to_string()),
        "idoklad.commit" => serde_json::to_value(
            idoklad::commit(
                pool,
                field(&payload, "content")?,
                field(&payload, "sha256")?,
                payload.get("source_sha256").and_then(Value::as_str),
            )
            .await?,
        )
        .map_err(|e| e.to_string()),
        "idoklad.source" => {
            serde_json::to_value(idoklad::source(pool, field(&payload, "invoice_id")?).await?)
                .map_err(|e| e.to_string())
        }
        "bank.match" => Ok(json!({"matched":match_bank(pool).await?})),
        "bank.link" => {
            link_bank(
                pool,
                field(&payload, "bank_id")?,
                field(&payload, "invoice_id")?,
            )
            .await?;
            Ok(json!({"ok":true}))
        }
        "exception.dismiss" => {
            dismiss_exception(pool, field(&payload, "id")?).await?;
            Ok(json!({"ok":true}))
        }
        "backup.export" => Ok(json!({"content":export_json(pool).await?})),
        "backup.restore" => {
            restore_json(pool, field(&payload, "content")?).await?;
            Ok(json!({"ok":true}))
        }
        "file.write" => {
            let path = field(&payload, "path")?;
            let content = field(&payload, "content")?;
            std::fs::write(path, content).map_err(|e| e.to_string())?;
            Ok(json!({"ok":true}))
        }
        "subject.lookup" => serde_json::to_value(
            registry::lookup(field(&payload, "kind")?, field(&payload, "identifier")?).await?,
        )
        .map_err(|e| e.to_string()),
        "demo.seed" => {
            seed_demo(pool).await?;
            Ok(json!({"ok":true}))
        }
        "demo.clear" => {
            clear_demo(pool).await?;
            Ok(json!({"ok":true}))
        }
        "tax.config" => tax_config(pool).await,
        "tax.configure" => save_tax_config(pool, payload).await,
        "tax.preview" => tax_preview(pool, field(&payload, "period")?).await,
        "tax.xml" => tax_xml(pool, field(&payload, "period")?).await,
        "tax.test" => tax_test(pool, field(&payload, "period")?).await,
        "annual.config" => annual::annual_config(pool, parse(&payload, "year")?).await,
        "annual.configure" => {
            annual::annual_save_config(pool, parse(&payload, "year")?, parse(&payload, "config")?)
                .await
        }
        "annual.summary" => annual::annual_summary(pool, parse(&payload, "year")?).await,
        "annual.csv" => {
            Ok(json!({"content":annual::annual_csv(pool, parse(&payload, "year")?).await?}))
        }
        _ => Err("Unknown action".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = std::env::var_os("INVOICE_DESKTOP_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or(app.path().app_data_dir().map_err(|e| e.to_string())?);
            let path = data_dir.join("invoices.sqlite");
            let db = tauri::async_runtime::block_on(open(&path)).map_err(|e| e.to_string())?;
            app.manage(Db(db.clone()));
            tauri::async_runtime::spawn(async move {
                let _ = background_tick(&db).await;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                    let _ = background_tick(&db).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![dispatch])
        .run(tauri::generate_context!())
        .expect("desktop app error");
}
