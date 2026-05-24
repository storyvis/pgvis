//! # Validation — checks plan constraints and synthesizes GROUP BY.

use super::planner::PlanConfig;
use super::types::*;
use crate::dialect::Dialect;
use crate::error::{Error, ErrorCode};
use crate::query_params::types::Operator;

/// Validate aggregate functions in the select list.
/// Returns error if aggregates are disabled.
/// Extracts aggregate info for GROUP BY synthesis.
pub fn validate_aggregates(
    selects: &[ResolvedSelect],
    config: &PlanConfig,
) -> Result<Vec<ResolvedAggregate>, Error> {
    let mut aggregates = Vec::new();

    for sel in selects {
        if let ResolvedSelect::Aggregate(agg) = sel {
            if !config.aggregates_enabled {
                return Err(Error::Plan {
                    message: format!(
                        "Aggregate function '{}' is disabled",
                        agg.function.sql_name()
                    ),
                    detail: Some(
                        "Set db-aggregates-enabled = true to enable aggregate functions"
                            .to_string(),
                    ),
                    hint: None,
                    code: ErrorCode::AggregatesDisabled,
                });
            }
            aggregates.push(agg.clone());
        }
    }

    Ok(aggregates)
}

/// Validate that the target table supports the requested mutation.
pub fn validate_mutation_target(
    table_info: &ResolvedTableInfo,
    table_name: &str,
    method: RequestMethod,
) -> Result<(), Error> {
    match method {
        RequestMethod::Post => {
            if !table_info.insertable {
                return Err(Error::Plan {
                    message: format!("Table '{table_name}' is not insertable"),
                    detail: Some("This may be a view without INSERT rules/triggers".to_string()),
                    hint: None,
                    code: ErrorCode::UnsupportedOperation,
                });
            }
        }
        RequestMethod::Patch | RequestMethod::Put => {
            if !table_info.updatable {
                return Err(Error::Plan {
                    message: format!("Table '{table_name}' is not updatable"),
                    detail: Some("This may be a view without UPDATE rules/triggers".to_string()),
                    hint: None,
                    code: ErrorCode::UnsupportedOperation,
                });
            }
        }
        RequestMethod::Delete => {
            if !table_info.deletable {
                return Err(Error::Plan {
                    message: format!("Table '{table_name}' is not deletable"),
                    detail: Some("This may be a view without DELETE rules/triggers".to_string()),
                    hint: None,
                    code: ErrorCode::UnsupportedOperation,
                });
            }
        }
        _ => {}
    }
    Ok(())
}

/// Validate that the dialect supports the features used in the request.
pub fn validate_dialect_support(request: &ApiRequest, dialect: &Dialect) -> Result<(), Error> {
    // Reject RPC calls on backends without routine support
    if request.is_rpc && !dialect.has_routines {
        return Err(Error::Unsupported(
            "RPC function calls are not supported on the current backend".to_string(),
        ));
    }

    // Reject array operators on backends that don't support them
    for filter in &request.filters {
        match filter.operator {
            Operator::Contains | Operator::ContainedBy if !dialect.supports_array_ops => {
                return Err(Error::Unsupported(format!(
                    "Array containment operator '{}' is not supported on this backend",
                    match filter.operator {
                        Operator::Contains => "cs",
                        Operator::ContainedBy => "cd",
                        _ => unreachable!(),
                    }
                )));
            }
            Operator::Overlap if !dialect.supports_array_ops => {
                return Err(Error::Unsupported(
                    "Overlap operator 'ov' is not supported on this backend".to_string(),
                ));
            }
            _ => {}
        }
    }

    Ok(())
}
