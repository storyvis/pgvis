//! # Parser for the `order=` query parameter.
//!
//! Parses PostgREST's order DSL: `name.desc.nullsfirst,age.asc`.
//! Also supports relation ordering: `clients(name).desc.nullsfirst`.
//!
//! Ports PostgREST's `pOrder` from `QueryParams.hs`.

use winnow::combinator::{alt, delimited, opt, preceded, separated};
use winnow::{Parser, Result};

use super::common::{field, field_name};
use super::types::{NullsOrder, OrderDirection, OrderTerm};
use crate::select_ast::JsonOperation;

/// An order item — either a direct field or a relation field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderItem {
    Term(OrderTerm),
    Relation(OrderRelationTerm),
}

/// Order by a field in an embedded relation: `clients(json_col->key).desc.nullsfirst`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRelationTerm {
    pub relation: String,
    pub field: String,
    pub json_path: Vec<JsonOperation>,
    pub direction: OrderDirection,
    pub nulls: Option<NullsOrder>,
}

/// Parse the full `order=` value.
pub fn parse_order(input: &str) -> Result<Vec<OrderItem>, String> {
    if input.is_empty() {
        return Ok(vec![]);
    }
    order_list
        .parse(input)
        .map_err(|e| format!("failed to parse order: {e}"))
}

fn order_list(input: &mut &str) -> Result<Vec<OrderItem>> {
    separated(1.., order_item, ',').parse_next(input)
}

fn order_item(input: &mut &str) -> Result<OrderItem> {
    // Relation form has `name(` — try it first (succeeds only if we see the `(`).
    alt((
        order_relation_term.map(OrderItem::Relation),
        order_term.map(OrderItem::Term),
    ))
    .parse_next(input)
}

fn order_term(input: &mut &str) -> Result<OrderTerm> {
    let (name, json_path) = field.parse_next(input)?;
    let direction = opt(preceded('.', direction)).parse_next(input)?;
    let nulls = opt(preceded('.', nulls)).parse_next(input)?;
    Ok(OrderTerm {
        field: name,
        json_path,
        direction: direction.unwrap_or_default(),
        nulls,
    })
}

fn order_relation_term(input: &mut &str) -> Result<OrderRelationTerm> {
    let relation = field_name.parse_next(input)?;
    let (col, json_path) = delimited('(', field, ')').parse_next(input)?;
    let direction = opt(preceded('.', direction)).parse_next(input)?;
    let nulls = opt(preceded('.', nulls)).parse_next(input)?;
    Ok(OrderRelationTerm {
        relation,
        field: col,
        json_path,
        direction: direction.unwrap_or_default(),
        nulls,
    })
}

fn direction(input: &mut &str) -> Result<OrderDirection> {
    alt((
        "asc".value(OrderDirection::Asc),
        "desc".value(OrderDirection::Desc),
    ))
    .parse_next(input)
}

fn nulls(input: &mut &str) -> Result<NullsOrder> {
    alt((
        "nullsfirst".value(NullsOrder::First),
        "nullslast".value(NullsOrder::Last),
    ))
    .parse_next(input)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select_ast::*;

    #[test]
    fn parse_simple_asc() {
        let result = parse_order("name.asc").unwrap();
        assert_eq!(result.len(), 1);
        if let OrderItem::Term(t) = &result[0] {
            assert_eq!(t.field, "name");
            assert_eq!(t.direction, OrderDirection::Asc);
            assert_eq!(t.nulls, None);
        } else {
            panic!("expected term");
        }
    }

    #[test]
    fn parse_desc_nullsfirst() {
        let result = parse_order("name.desc.nullsfirst").unwrap();
        if let OrderItem::Term(t) = &result[0] {
            assert_eq!(t.field, "name");
            assert_eq!(t.direction, OrderDirection::Desc);
            assert_eq!(t.nulls, Some(NullsOrder::First));
        } else {
            panic!("expected term");
        }
    }

    #[test]
    fn parse_json_order() {
        let result = parse_order("json_col->key.asc.nullslast").unwrap();
        if let OrderItem::Term(t) = &result[0] {
            assert_eq!(t.field, "json_col");
            assert_eq!(t.json_path.len(), 1);
            assert_eq!(
                t.json_path[0],
                JsonOperation::Arrow(JsonOperand::Key("key".into()))
            );
            assert_eq!(t.direction, OrderDirection::Asc);
            assert_eq!(t.nulls, Some(NullsOrder::Last));
        } else {
            panic!("expected term");
        }
    }

    #[test]
    fn parse_relation_order() {
        let result = parse_order("clients(json_col->key).desc.nullsfirst").unwrap();
        if let OrderItem::Relation(r) = &result[0] {
            assert_eq!(r.relation, "clients");
            assert_eq!(r.field, "json_col");
            assert_eq!(
                r.json_path[0],
                JsonOperation::Arrow(JsonOperand::Key("key".into()))
            );
            assert_eq!(r.direction, OrderDirection::Desc);
            assert_eq!(r.nulls, Some(NullsOrder::First));
        } else {
            panic!("expected relation term, got {:?}", result[0]);
        }
    }

    #[test]
    fn parse_multiple() {
        let result = parse_order("name,clients(name),id").unwrap();
        assert_eq!(result.len(), 3);
        assert!(matches!(&result[0], OrderItem::Term(t) if t.field == "name"));
        assert!(matches!(&result[1], OrderItem::Relation(r) if r.relation == "clients"));
        assert!(matches!(&result[2], OrderItem::Term(t) if t.field == "id"));
    }

    #[test]
    fn parse_no_direction() {
        let result = parse_order("name").unwrap();
        if let OrderItem::Term(t) = &result[0] {
            assert_eq!(t.field, "name");
            assert_eq!(t.direction, OrderDirection::Asc); // default
            assert_eq!(t.nulls, None);
        } else {
            panic!("expected term");
        }
    }

    #[test]
    fn parse_empty() {
        let result = parse_order("").unwrap();
        assert!(result.is_empty());
    }
}
