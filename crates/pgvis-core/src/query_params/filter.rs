//! # Filter expression parser.
//!
//! Parses PostgREST operator expressions like `age=gte.18`, `name=eq.John`,
//! `status=in.(active,pending)`, `search=fts(english).hello`.
//!
//! Ports PostgREST's `pOpExpr` from `QueryParams.hs`.
//!
//! The grammar is shared with the boolean logic tree ([`super::logic`]) —
//! only the value-terminator differs. [`op_body`] is parameterised on a
//! single-value parser so both callers reuse the same operator dispatch.

use winnow::combinator::{delimited, opt};
use winnow::error::ContextError;
use winnow::{Parser, Result};

use super::common::{
    dot, field, fts_lang, fts_op, quant_operator, quantifier, quoted_value, simple_operator,
};
use super::types::{Filter, FilterValue, IsKind, Operator, Quantifier};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a filter from a query parameter key/value pair.
///
/// - `key`   = column name (possibly with json path), e.g. `data->key`
/// - `value` = operator expression, e.g. `eq.hello`, `not.in.(1,2,3)`
pub fn parse_filter(key: &str, value: &str) -> Result<Filter, String> {
    let (field_name, json_path) = field
        .parse(key)
        .map_err(|e| format!("invalid filter key '{key}': {e}"))?;

    let (negate, op, quant, fv) = (|input: &mut &str| op_expr(ValueCtx::Top, input))
        .parse(value)
        .map_err(|e| format!("invalid filter for '{key}': {e}"))?;

    Ok(Filter {
        field: field_name,
        json_path,
        operator: op,
        negate,
        quantifier: quant,
        value: fv,
    })
}

/// Parse a logic-tree filter item like `field.eq.value` where the value
/// terminates at `,` or `)`.
///
/// Used by [`super::logic`] to keep operator-grammar in one place.
pub fn parse_logic_filter(input: &mut &str) -> Result<Filter> {
    let (field_name, json_path) = field.parse_next(input)?;
    dot.parse_next(input)?;
    let (negate, op, quant, fv) = op_expr(ValueCtx::Logic, input)?;
    Ok(Filter {
        field: field_name,
        json_path,
        operator: op,
        negate,
        quantifier: quant,
        value: fv,
    })
}

// ---------------------------------------------------------------------------
// Shared op-expr grammar — `ctx` selects the scalar value terminator.
// ---------------------------------------------------------------------------

/// Whether a scalar value runs to EOF (top-level filter) or stops at `,`/`)`
/// (inside a logic tree).
#[derive(Clone, Copy)]
enum ValueCtx {
    Top,
    Logic,
}

/// `[not.]op[(quant)].value`.
fn op_expr(
    ctx: ValueCtx,
    input: &mut &str,
) -> Result<(bool, Operator, Option<Quantifier>, FilterValue)> {
    let negate = opt(("not", '.')).parse_next(input)?.is_some();
    let (op, quant, fv) = op_body(ctx, input)?;
    Ok((negate, op, quant, fv))
}

/// Dispatch on the operator keyword and parse the value with the right shape.
///
/// Distinctive shapes (IN, IS, ISDISTINCT, FTS-family) come first so we don't
/// mis-parse `isdistinct` as `is` or `phfts` as `fts`.
fn op_body(
    ctx: ValueCtx,
    input: &mut &str,
) -> Result<(Operator, Option<Quantifier>, FilterValue)> {
    if input.starts_with("in.") {
        "in".parse_next(input)?;
        dot.parse_next(input)?;
        let vals = in_list(input)?;
        return Ok((Operator::In, None, FilterValue::List(vals)));
    }
    if input.starts_with("isdistinct.") {
        "isdistinct".parse_next(input)?;
        dot.parse_next(input)?;
        let v = value(ctx, input)?;
        return Ok((Operator::IsDistinct, None, FilterValue::Single(v)));
    }
    if input.starts_with("is.") {
        "is".parse_next(input)?;
        dot.parse_next(input)?;
        let v = value(ctx, input)?;
        let kind = is_kind_from(&v)?;
        return Ok((Operator::Is, None, FilterValue::Is(kind)));
    }
    if input.starts_with("fts")
        || input.starts_with("plfts")
        || input.starts_with("phfts")
        || input.starts_with("wfts")
    {
        let ctor = fts_op.parse_next(input)?;
        let lang = opt(fts_lang).parse_next(input)?;
        dot.parse_next(input)?;
        let v = value(ctx, input)?;
        return Ok((ctor(lang), None, FilterValue::Single(v)));
    }
    if let Some(op) = opt(quant_operator).parse_next(input)? {
        let q = opt(quantifier).parse_next(input)?;
        dot.parse_next(input)?;
        let v = value(ctx, input)?;
        return Ok((op, q, FilterValue::Single(v)));
    }
    // Simple operators: cs, cd, ov, sl, sr, nxr, nxl, adj.
    let op = simple_operator.parse_next(input)?;
    dot.parse_next(input)?;
    let v = value(ctx, input)?;
    Ok((op, None, FilterValue::Single(v)))
}

fn is_kind_from(s: &str) -> Result<IsKind> {
    match s {
        "null" => Ok(IsKind::Null),
        "notnull" => Ok(IsKind::NotNull),
        "true" => Ok(IsKind::True),
        "false" => Ok(IsKind::False),
        "unknown" => Ok(IsKind::Unknown),
        _ => Err(ContextError::new()),
    }
}

// ---------------------------------------------------------------------------
// Scalar / list / array value parsers
// ---------------------------------------------------------------------------

/// A bare scalar value. Quoted `"…"` and `{…}` array literals are honoured in
/// both contexts; otherwise the terminator depends on `ctx`.
fn value(ctx: ValueCtx, input: &mut &str) -> Result<String> {
    if input.starts_with('"') {
        return quoted_value(input);
    }
    if input.starts_with('{') {
        return pg_array(input);
    }
    let end = match ctx {
        ValueCtx::Top => input.len(),
        ValueCtx::Logic => input.find([',', ')']).unwrap_or(input.len()),
    };
    let (consumed, rest) = input.split_at(end);
    let out = consumed.to_string();
    *input = rest;
    Ok(out)
}

/// `(a,b,"c,d")` — quote-aware comma split, no nested parens permitted.
fn in_list(input: &mut &str) -> Result<Vec<String>> {
    delimited('(', list_body, ')').parse_next(input)
}

fn list_body(input: &mut &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escape_next = false;
    let mut consumed = 0usize;

    for (i, ch) in input.char_indices() {
        if !in_quotes && ch == ')' {
            consumed = i;
            values.push(std::mem::take(&mut current));
            *input = &input[consumed..];
            return Ok(values);
        }
        consumed = i + ch.len_utf8();
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escape_next = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => values.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    // No closing `)` found.
    *input = &input[consumed..];
    Err(ContextError::new())
}

/// Postgres array literal `{…}` with nested-brace counting.
fn pg_array(input: &mut &str) -> Result<String> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'{') {
        return Err(ContextError::new());
    }
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    let out = input[..=i].to_string();
                    *input = &input[i + 1..];
                    return Ok(out);
                }
            }
            _ => {}
        }
    }
    Err(ContextError::new())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn single(f: &Filter) -> &str {
        match &f.value {
            FilterValue::Single(s) => s,
            _ => panic!("expected Single, got {:?}", f.value),
        }
    }

    #[test]
    fn parse_eq() {
        let f = parse_filter("name", "eq.John").unwrap();
        assert_eq!(f.field, "name");
        assert_eq!(f.operator, Operator::Eq);
        assert_eq!(single(&f), "John");
        assert!(!f.negate);
        assert!(f.json_path.is_empty());
    }

    #[test]
    fn parse_not_eq() {
        let f = parse_filter("name", "not.eq.John").unwrap();
        assert!(f.negate);
        assert_eq!(f.operator, Operator::Eq);
        assert_eq!(single(&f), "John");
    }

    #[test]
    fn parse_gte() {
        let f = parse_filter("age", "gte.18").unwrap();
        assert_eq!(f.operator, Operator::Gte);
        assert_eq!(single(&f), "18");
    }

    #[test]
    fn parse_in() {
        let f = parse_filter("status", "in.(active,pending)").unwrap();
        assert_eq!(f.operator, Operator::In);
        match &f.value {
            FilterValue::List(v) => {
                assert_eq!(v, &vec!["active".to_string(), "pending".to_string()]);
            }
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_in_quoted_comma() {
        // Quotes are escape syntax (so an embedded `,` doesn't split the list);
        // they are not part of the value itself.
        let f = parse_filter("status", "in.(a,\"b,c\",d)").unwrap();
        match &f.value {
            FilterValue::List(v) => assert_eq!(
                v,
                &vec!["a".to_string(), "b,c".to_string(), "d".to_string()]
            ),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_is_null() {
        let f = parse_filter("deleted_at", "is.null").unwrap();
        assert_eq!(f.operator, Operator::Is);
        assert_eq!(f.value, FilterValue::Is(IsKind::Null));
    }

    #[test]
    fn parse_is_notnull() {
        let f = parse_filter("deleted_at", "is.notnull").unwrap();
        assert_eq!(f.value, FilterValue::Is(IsKind::NotNull));
    }

    #[test]
    fn parse_is_distinct() {
        let f = parse_filter("col", "isdistinct.value").unwrap();
        assert_eq!(f.operator, Operator::IsDistinct);
        assert_eq!(single(&f), "value");
    }

    #[test]
    fn parse_fts() {
        let f = parse_filter("body", "fts.hello").unwrap();
        assert_eq!(f.operator, Operator::Fts(None));
        assert_eq!(single(&f), "hello");
    }

    #[test]
    fn parse_fts_with_lang() {
        let f = parse_filter("body", "fts(english).hello world").unwrap();
        assert_eq!(f.operator, Operator::Fts(Some("english".into())));
        assert_eq!(single(&f), "hello world");
    }

    #[test]
    fn parse_plfts() {
        let f = parse_filter("body", "plfts.hello").unwrap();
        assert_eq!(f.operator, Operator::PlainFts(None));
    }

    #[test]
    fn parse_phfts() {
        let f = parse_filter("body", "phfts.hello").unwrap();
        assert_eq!(f.operator, Operator::PhraseFts(None));
    }

    #[test]
    fn parse_wfts() {
        let f = parse_filter("body", "wfts.hello").unwrap();
        assert_eq!(f.operator, Operator::WebFts(None));
    }

    #[test]
    fn parse_contains() {
        let f = parse_filter("tags", "cs.{a,b}").unwrap();
        assert_eq!(f.operator, Operator::Contains);
        assert_eq!(single(&f), "{a,b}");
    }

    #[test]
    fn parse_quantifier_any() {
        let f = parse_filter("age", "eq(any).18").unwrap();
        assert_eq!(f.operator, Operator::Eq);
        assert_eq!(f.quantifier, Some(Quantifier::Any));
        assert_eq!(single(&f), "18");
    }

    #[test]
    fn parse_quantifier_all() {
        let f = parse_filter("age", "gte(all).21").unwrap();
        assert_eq!(f.operator, Operator::Gte);
        assert_eq!(f.quantifier, Some(Quantifier::All));
    }

    #[test]
    fn parse_not_quantifier() {
        let f = parse_filter("age", "not.eq(all).18").unwrap();
        assert!(f.negate);
        assert_eq!(f.operator, Operator::Eq);
        assert_eq!(f.quantifier, Some(Quantifier::All));
    }

    #[test]
    fn parse_json_filter_key() {
        use crate::select_ast::{JsonOperand, JsonOperation};
        let f = parse_filter("data->key", "eq.hello").unwrap();
        assert_eq!(f.field, "data");
        assert_eq!(
            f.json_path,
            vec![JsonOperation::Arrow(JsonOperand::Key("key".into()))]
        );
        assert_eq!(single(&f), "hello");
    }

    #[test]
    fn parse_error_unknown_op() {
        assert!(parse_filter("name", "wat.John").is_err());
    }

    #[test]
    fn parse_error_unterminated_quote() {
        assert!(parse_filter("name", "eq.\"unterm").is_err());
    }
}
