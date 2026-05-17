# 06 — Errors, Configuration, Preferences

The three cross-cutting concerns that every surface and the core share.
**Status: `[Implemented]`** for the types and parsing. The REST surface now
consumes them at the execute boundary; MCP does not yet (no backend wired).

## Errors

Module [pgvis-core/src/error.rs](../crates/pgvis-core/src/error.rs). One unified
`Error` enum plus a machine-readable `ErrorCode`, both PostgREST-compatible so
existing clients (e.g. `supabase-js`, `postgrest-py`) handle pgvis errors
unchanged.

### Shape

Every error serializes to the PostgREST JSON shape:

```json
{ "code": "PGRST200", "message": "...", "details": "...", "hint": "..." }
```

`Error` variants: `Introspection`, `Execution { message, db_code, detail, hint }`,
`Parse { message, detail, code }`, `Plan { message, detail, hint, code }`,
`Config`, `Auth { message, code }`, `Unsupported`, `Internal`. Convenience
constructors (`invalid_select`, `invalid_filter`, `ambiguous_relationship`,
`not_found`, `unsupported`) build the right `code` + `hint` so call sites stay
terse and messages stay actionable.

### Codes and HTTP mapping

`ErrorCode::as_str()` yields the `PGRST*` (or pgvis-specific `PGV*`) string;
`http_status()` yields the status. Ranges:

| Range | Category | Examples |
| ------- | ---------- | ---------- |
| `PGRST0xx` | connection/pool | `PGRST000` → 503 |
| `PGRST1xx` | parse errors | `PGRST100` invalid select → 400 |
| `PGRST2xx` | plan/schema resolution | `PGRST200` ambiguous relationship → 300; `PGRST204` not found → 404 |
| `PGRST3xx` | auth | `PGRST300` JWT missing → 401; `PGRST303` insufficient privilege → 403 |
| `PGRST4xx` | execution | `PGRST400` db error → 500; `PGRST401` timeout → 504 |
| `PGV*` | pgvis-specific | `PGV001` unsupported op → 400; `PGV500` internal → 500 |

`Error::http_status()` additionally special-cases the database `db_code` on
`Execution` errors so constraint/permission failures map precisely — e.g.
`23505` unique violation → 409, `42501` insufficient privilege → 403, `42P01`
undefined table → 404 — before falling back to the code's default status.

Adapters consume this directly: the REST handler sets the response status from
`err.http_status()` and the body from `err.code().as_str()` + `err.to_string()`
([routing.rs](../crates/pgvis-router/src/routing.rs)); MCP returns
`[CODE] message` ([tools.rs](../crates/pgvis-mcp/src/tools.rs)). Plan-time
dialect rejections surface here as `Unsupported`/`PGV001`
([03-backends-and-dialects.md](03-backends-and-dialects.md)).

## Configuration

Module [pgvis-core/src/config.rs](../crates/pgvis-core/src/config.rs). pgvis
deliberately *splits* config rather than using one flat struct: `Config` is the
shared inner config every crate agrees on; backend pool settings, REST bind/CORS
settings, and CLI/figment layering live in their respective crates so a library
consumer only sees what is relevant.

### `Config`

Shared knobs, each mapped to its PostgREST equivalent in the source doc-comments:

- **Schema selection** — `schemas`, `extra_search_path`.
- **Auth** — `jwt_secret`, `jwt_algo` (`HS*`/`RS256`/`EdDSA`), `anon_role`,
  `role_claim_key`.
- **Feature gates** — `aggregates_enabled`, `plan_enabled` (EXPLAIN media type),
  `tx_allow_override`, `tx_rollback_all`.
- **Query limits** — `max_rows` (server-side cap applied in the plan layer via
  `PlanConfig`), `statement_timeout_ms`.
- **Hooks** — `pre_request` (function called after role switch, before the main
  query; can abort the request).
- **OpenAPI** — `openapi_title`, `openapi_server_url`, `openapi_mode`
  (`IgnorePrivileges` default / `FollowPrivileges` / `Disabled`).
- **Routing** — a nested `RoutingConfig`.

### `RoutingConfig`

Controls URL structure *and* MCP tool naming from one place so the two surfaces
stay parallel:

- `prefix` — route prefix (`"api"` default, `""` for PostgREST compat).
- `schema_in_path` — `true` → `/{prefix}/{schema}/{table}`; `false` → schema
  from `Accept-Profile`/`Content-Profile` header or `default_schema`.
- `default_schema` — used when `schema_in_path = false`.
- `mcp_separator` — char joining schema and verb in MCP tool names (`/` default).
- Helpers: `mcp_tool_name(schema, verb, target)`, `schema_path_prefix(schema)`,
  `normalized_prefix()` — used by both [routing.rs](../crates/pgvis-router/src/routing.rs)
  and [tools.rs](../crates/pgvis-mcp/src/tools.rs). The routing modes are
  tabulated in [04-surfaces.md](04-surfaces.md).

### Layering

The standalone binary layers config via `figment` (TOML file → `PGVIS_*` env →
CLI args → built-in `Default`) and `clap`
([pgvis-server/src/main.rs](../crates/pgvis-server/src/main.rs)). All `Config`
fields use `#[serde(default = ...)]` so partial files are valid.

## Preferences (the `Prefer` header)

Module [pgvis-core/src/preferences.rs](../crates/pgvis-core/src/preferences.rs).
`Preferences::parse(header) -> (Preferences, Vec<String>)` returns typed
preferences plus a list of unrecognized tokens (for `handling=strict`
validation). `applied_header()` produces the `Preference-Applied` response
echoing only honored preferences. RFC 7240 comma-separated and repeated headers
are both accepted.

| `Prefer` token | Type | Consumed by |
| ---------------- | ------ | ------------- |
| `return=representation\|minimal\|headers-only\|none` | `PreferReturn` | mutation response shape |
| `count=exact\|planned\|estimated` | `PreferCount` | `CountStrategy` in the plan; CTE `total_count` (estimated needs `supports_estimated_count`) |
| `resolution=merge-duplicates\|ignore-duplicates` | `PreferResolution` | upsert `ON CONFLICT` (`ResolvedConflict`) |
| `handling=strict\|lenient` | `PreferHandling` | whether unknown prefs 400 |
| `timezone=<tz>` | `String` | session timezone (needs `supports_set_timezone`) |
| `missing=default\|null` | `PreferMissing` | omitted-column behavior on write |
| `tx=commit\|rollback` | `PreferTx` | `ExecContext.tx_end`, gated by `Config::tx_allow_override` |
| `max-affected=N` | `u64` | post-mutation row-count guard |
| `params=single-object\|multiple-objects` | `PreferParams` | RPC argument passing |

`Preferences` flows unchanged through `ApiRequest` into every `ActionPlan`
variant ([02-core-pipeline.md](02-core-pipeline.md)); the dialect gates marked
above are enforced in the plan layer
([03-backends-and-dialects.md](03-backends-and-dialects.md)). On REST these now
take effect: `tx`, `statement_timeout`, role/claims, and count flow through
`ExecContext` into the Postgres `execute` path
([execute.rs](../crates/pgvis-postgres/src/execute.rs)). MCP applies them only
once a backend is wired into its server — [08-future-scope.md](08-future-scope.md).
