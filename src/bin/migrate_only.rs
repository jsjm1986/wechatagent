//! Explicit migration-only maintenance entry point.
//!
//! This binary never loads `.env`, starts HTTP listeners, or launches workers.
//! It exists for rehearsing and applying the exact startup migration/index path
//! against an explicitly named database.

use std::env;

use wechatagent::db::{migrations, Database};

const FORBIDDEN_DATABASES: &[&str] = &["admin", "config", "local"];

fn required_arg(name: &str) -> anyhow::Result<String> {
    let prefix = format!("--{name}=");
    env::args()
        .skip(1)
        .find_map(|arg| arg.strip_prefix(&prefix).map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing required argument {prefix}<value>"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let uri = required_arg("uri")?;
    let database = required_arg("database")?;
    let confirm = required_arg("confirm")?;

    if FORBIDDEN_DATABASES.contains(&database.as_str()) {
        anyhow::bail!("refusing system database {database}");
    }
    let expected = format!("migrate-only:{database}");
    if confirm != expected {
        anyhow::bail!("confirmation mismatch; expected --confirm={expected}");
    }

    let db = Database::connect(&uri, &database).await?;
    migrations::run(&db).await?;
    db.ensure_indexes().await?;
    println!("migration-only completed for database={database}");
    Ok(())
}
