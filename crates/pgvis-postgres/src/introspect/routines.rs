//! Stored functions/procedures introspection query and row decoder.

use indexmap::IndexMap;
use pgvis_core::cache::{IsolationLevel, QualifiedIdentifier, Routine, RoutineParam, Volatility};
use pgvis_core::error::Error;
use serde::Deserialize;
use tokio_postgres::types::Type;
use tokio_postgres::Client;

/// SQL query for routines introspection (loaded at compile time).
const ROUTINES_SQL: &str = include_str!("../sql/routines.sql");

/// Intermediate struct for deserialising function parameters from JSON.
#[derive(Debug, Deserialize)]
struct ParamJson {
    name: String,
    #[serde(rename = "type")]
    typ: String,
    required: bool,
    is_variadic: bool,
}

/// Intermediate struct for deserialising function settings from JSON.
#[derive(Debug, Deserialize)]
struct SettingJson {
    key: String,
    value: String,
}

/// Query all stored functions in the given schemas.
///
/// Returns an ordered map of `QualifiedIdentifier → Vec<Routine>` (Vec for overloads).
pub async fn query_routines(
    client: &Client,
    schemas: &[String],
) -> Result<IndexMap<QualifiedIdentifier, Vec<Routine>>, Error> {
    // For now, pass empty array for hoisted settings (will be configurable later)
    let hoisted_settings: Vec<String> = Vec::new();

    // Use prepare_typed to explicitly tell Postgres params are TEXT[].
    // Without this, Postgres infers regnamespace[] from the cast in the SQL.
    let stmt = client
        .prepare_typed(ROUTINES_SQL, &[Type::TEXT_ARRAY, Type::TEXT_ARRAY])
        .await
        .map_err(|e| Error::Introspection(format!("routines query prepare failed: {e}")))?;
    let rows = client
        .query(&stmt, &[&schemas, &hoisted_settings])
        .await
        .map_err(|e| Error::Introspection(format!("routines query failed: {e}")))?;

    let mut routines: IndexMap<QualifiedIdentifier, Vec<Routine>> = IndexMap::new();

    for row in &rows {
        let schema: String = row.get("proc_schema");
        let name: String = row.get("proc_name");
        let description: Option<String> = row.get("proc_description");

        // Decode params from JSON
        let params_json: serde_json::Value = row.get("params");
        let param_rows: Vec<ParamJson> = serde_json::from_value(params_json)
            .map_err(|e| Error::Introspection(format!("failed to decode params for {schema}.{name}: {e}")))?;

        let params: Vec<RoutineParam> = param_rows
            .into_iter()
            .map(|p| RoutineParam {
                name: p.name,
                typ: p.typ,
                required: p.required,
                is_variadic: p.is_variadic,
            })
            .collect();

        // Return type info
        let return_type_schema: String = row.get("return_type_schema");
        let return_type_name: String = row.get("return_type_name");
        let rettype_is_setof: bool = row.get("rettype_is_setof");
        let rettype_is_composite: bool = row.get("rettype_is_composite");

        // Volatility
        let volatility_char: String = row.get("volatility");
        let volatility = match volatility_char.as_str() {
            "i" => Volatility::Immutable,
            "s" => Volatility::Stable,
            _ => Volatility::Volatile,
        };

        let has_variadic: bool = row.get("has_variadic");

        // Transaction isolation level
        let isolation_str: Option<String> = row.get("transaction_isolation_level");
        let isolation_level = isolation_str.and_then(|s| match s.trim() {
            "read committed" => Some(IsolationLevel::ReadCommitted),
            "repeatable read" => Some(IsolationLevel::RepeatableRead),
            "serializable" => Some(IsolationLevel::Serializable),
            _ => None,
        });

        // Function settings
        let settings_json: serde_json::Value = row.get("settings");
        let setting_rows: Vec<SettingJson> = serde_json::from_value(settings_json)
            .unwrap_or_default();
        let settings: Vec<(String, String)> = setting_rows
            .into_iter()
            .map(|s| (s.key, s.value))
            .collect();

        let return_type = if return_type_schema == "pg_catalog" {
            return_type_name.clone()
        } else {
            format!("{return_type_schema}.{return_type_name}")
        };

        let ident = QualifiedIdentifier::new(&schema, &name);

        let routine = Routine {
            ident: ident.clone(),
            description,
            params,
            return_type,
            return_type_is_set: rettype_is_setof,
            return_type_is_composite: rettype_is_composite,
            volatility,
            is_variadic: has_variadic,
            isolation_level,
            settings,
        };

        routines.entry(ident).or_default().push(routine);
    }

    Ok(routines)
}
