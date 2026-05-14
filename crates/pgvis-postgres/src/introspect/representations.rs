//! Data representations (domain type casts) introspection query and decoder.

use std::collections::HashMap;

use pgvis_core::cache::DataRepresentation;
use pgvis_core::error::Error;
use tokio_postgres::Client;

/// SQL query for data representations introspection (loaded at compile time).
const REPRESENTATIONS_SQL: &str = include_str!("../sql/representations.sql");

/// Query all data representation casts (domain → json/text and vice versa).
///
/// Returns a map of `source_type → Vec<DataRepresentation>`.
pub async fn query_representations(
    client: &Client,
) -> Result<HashMap<String, Vec<DataRepresentation>>, Error> {
    let rows = client
        .query(REPRESENTATIONS_SQL, &[])
        .await
        .map_err(|e| Error::Introspection(format!("representations query failed: {e}")))?;

    let mut reps: HashMap<String, Vec<DataRepresentation>> = HashMap::new();

    for row in &rows {
        let source_type: String = row.get("source_type");
        let target_type: String = row.get("target_type");
        let function_name: String = row.get("function_name");

        // Extract schema from qualified function name (e.g. "public.my_func" → "public")
        let function_schema = function_name
            .split('.')
            .next()
            .unwrap_or("public")
            .to_string();

        let rep = DataRepresentation {
            source_type: source_type.clone(),
            target_type,
            function_name,
            function_schema,
        };

        reps.entry(source_type).or_default().push(rep);
    }

    Ok(reps)
}
