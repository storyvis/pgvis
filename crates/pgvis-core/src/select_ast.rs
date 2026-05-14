//! # Select item AST — the parsed representation of the `select` parameter.
//!
//! These types are the **output** of the winnow parser and the **input** to the plan layer.
//! They are backend-agnostic — the same AST feeds REST, MCP, and embed surfaces.
//!
//! ## PostgREST Equivalent
//!
//! Maps to `FieldName`, `SelectItem`, `JsonOperation` in `ApiRequest/Types.hs`.
//!
//! ## Grammar (informally)
//!
//! ```text
//! select     = field ("," field)*
//! field      = [alias ":"] (spread | relation | column)
//! column     = name [json_path] ["::cast"] [".agg()" ["::cast"]]
//! relation   = name ["!" hint] ["!" join] "(" select ")"
//! spread     = "..." relation
//! json_path  = ("->" key | "->>" key)*
//! key        = identifier | integer
//! agg        = "sum" | "avg" | "max" | "min" | "count"
//! ```

use serde::{Deserialize, Serialize};

/// A single item in the parsed `select` parameter.
///
/// The parser produces `Vec<SelectItem>` which the plan layer walks to build
/// a `ReadPlanTree`. Each variant maps to different SQL generation:
///
/// - `Field` → a column reference (possibly with cast, JSON path, aggregate)
/// - `Relation` → an embedded resource (becomes `json_agg` subquery)
/// - `Spread` → a to-one relation whose columns are "spread" to the parent level
/// - `Star` → all columns of the current table
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectItem {
    /// A column field: `alias:col->json::cast.agg()::agg_cast`
    Field(FieldSelect),

    /// An embedded relation: `alias:relation!hint!inner(subselect)`
    Relation(RelationSelect),

    /// A spread relation (to-one only): `...relation!hint(subselect)`
    Spread(SpreadSelect),

    /// `*` — all columns of the current table.
    Star,
}

/// A column field selection with optional JSON path, cast, and aggregate.
///
/// Examples:
/// - `name` → just the column
/// - `alias:name` → aliased column
/// - `data->>'key'` → JSON extraction
/// - `amount.sum()` → aggregate
/// - `total:amount::numeric.sum()::text` → full form
/// - `count()` → row count (name is empty)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSelect {
    /// Column name (empty for bare `count()`).
    pub name: String,

    /// Optional alias (`alias:column`).
    pub alias: Option<String>,

    /// JSON path operations (`->key`, `->>'key'`, `->0`).
    pub json_path: Vec<JsonOperation>,

    /// Cast applied to the column before aggregation (`::type`).
    pub cast: Option<String>,

    /// Aggregate function applied to the column.
    pub aggregate: Option<AggregateFunction>,

    /// Cast applied after aggregation (`.sum()::text`).
    pub aggregate_cast: Option<String>,
}

/// An embedded resource selection.
///
/// Example: `orders!customer_id_fkey!inner(id,total,items(name))`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationSelect {
    /// Relation (table/view) name.
    pub name: String,

    /// Optional alias.
    pub alias: Option<String>,

    /// Disambiguation hint (FK constraint name or table name).
    pub hint: Option<String>,

    /// Join type override (default is LEFT).
    pub join_type: Option<JoinType>,

    /// Nested select items within this relation.
    pub children: Vec<SelectItem>,
}

/// A spread relation — to-one relation whose columns are pulled up to the parent.
///
/// Example: `...customers!inner(name, subscription_date.max())`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpreadSelect {
    /// Relation (table/view) name.
    pub name: String,

    /// Disambiguation hint.
    pub hint: Option<String>,

    /// Join type override.
    pub join_type: Option<JoinType>,

    /// Nested select items.
    pub children: Vec<SelectItem>,
}

/// Join type for embedded resources.
///
/// Controls whether the parent row is excluded when the embedded resource
/// has no matching rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JoinType {
    /// `!inner` — exclude parent rows with no matching children (INNER JOIN).
    Inner,
    /// `!left` — include parent rows even with no children (LEFT JOIN, default).
    Left,
}

/// JSON path operation — `->` (object/array access) or `->>` (text extraction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JsonOperation {
    /// `->key` or `->index` — returns JSON.
    Arrow(JsonOperand),
    /// `->>key` or `->>index` — returns text.
    DoubleArrow(JsonOperand),
}

/// The operand of a JSON path operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum JsonOperand {
    /// Object key access: `->'field_name'` or `->'field_name'`.
    Key(String),
    /// Array index access: `->0`, `->1`, etc.
    Index(i64),
}

/// Supported aggregate functions.
///
/// Gated by `Config::aggregates_enabled` AND `Dialect::supports_aggregates`.
///
/// PostgREST equivalent: the aggregate functions in `pFieldSelect` parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AggregateFunction {
    /// `sum(column)` — total.
    Sum,
    /// `avg(column)` — average.
    Avg,
    /// `max(column)` — maximum.
    Max,
    /// `min(column)` — minimum.
    Min,
    /// `count()` — row count. Special: does not require a column.
    Count,
}

impl AggregateFunction {
    /// Parse an aggregate function name.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sum" => Some(Self::Sum),
            "avg" => Some(Self::Avg),
            "max" => Some(Self::Max),
            "min" => Some(Self::Min),
            "count" => Some(Self::Count),
            _ => None,
        }
    }

    /// The SQL function name.
    pub fn sql_name(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Max => "max",
            Self::Min => "min",
            Self::Count => "count",
        }
    }
}
