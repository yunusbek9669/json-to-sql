# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Purpose

**json-to-sql** is a Universal Adaptive Query (UAQ) Engine — a Rust library that compiles declarative JSON queries into safe, parameterized PostgreSQL SQL. It is compiled as a `cdylib` for FFI integration with PHP, Java, Node.js, and Python. Security is a primary constraint: every table/column access is validated against a caller-supplied whitelist, and all values become named parameters (`:p1`, `:p2`, ...).

## Build & Test Commands

```bash
# Build shared library (.so / .dylib / .dll)
cargo build --release

# Run all tests
cargo test

# Run a specific test by name
cargo test test_compact_format

# Debug build
cargo build
```

Test files live in `tests/` (integration, strict-mode, mixed-mode, manual) and `src/bin/test_inputs.rs`.

## Architecture

### Data Flow

```
C FFI call: uaq_parse(json, whitelist, relations, macros)
  → api.rs          UTF-8 validation, FFI marshaling
  → parser.rs       Parses JSON into a QueryNode tree
  → generator/      Converts tree to SQL + params map
      mod.rs         Orchestrates, owns mutable state (SqlGenerator)
      processor.rs   Recursive node → SQL fragment builder (largest file)
      relation.rs    BFS graph for auto-join path discovery
      condition.rs   WHERE clause + parameterization
  → guard/          Security layer called throughout generation
      mod.rs         Whitelist enforcement, alias resolution, field mapping
      validator.rs   Regex + allowed-function checks on expressions
      threats.rs     Global keyword blocklist (DROP, UNION, etc.)
      formatter.rs   Output formatting, timezone, mapped-field expansion
  → ParseResult     { isOk, sql, params, message }
```

### Key Types (`models.rs`)

- `QueryNode` — tree node with name, `is_list`, `SourceDef`, `fields`, and `children`
- `SourceDef` — table name, `FilterRule` list, limit/offset/order/join-type
- `FilterRule` — field, operator (`eq`/`neq`/`gt`/`lt`/`like`/`in`/`between`), value
- `ParseResult` — final output wrapper

### SqlGenerator State (`generator/mod.rs`)

Mutable struct that accumulates SQL fragments during tree traversal:
- `param_counter` — drives `:p1`, `:p2`, ... unique names
- `joined_aliases` — prevents duplicate JOINs when multiple children share a table
- `relation_graph` — cached for BFS pathfinding across multi-hop relations

### Important Patterns

**Relation keys** use directional operators: `A->B`, `A<-B`, `A-><-B`, `A<->B`, with optional `:node_name` suffix. Templates use `@join`, `@table`, `@1`, `@2` as placeholders.

**Macros** are reusable backend-defined query templates. The parser merges a macro definition with caller-supplied overrides when `@source` names a macro.

**Flattening** (`@flatten: true`) merges a 1-to-1 child node's fields into the parent JSON object instead of nesting.

**Inline aggregations** in `@fields` use syntax like `"count": "COUNT()@table[]"` and emit correlated subqueries.

**Join priority**: explicit non-macro join > macro join > default — resolved in `processor.rs` when multiple children target the same table.

### Security Validation Order

1. `threats.rs` — global blocklist checked first on all input strings
2. `guard/mod.rs` — table/column existence against whitelist
3. `validator.rs` — expression format, allowed-function whitelist (~50 PostgreSQL functions)
4. All user values become parameters; `SELECT` inside `@fields` is rejected

## Request / Response Contract

**Input JSON** (passed as C string):
```json
{
  "@data[]": {
    "@source": "table[field: value, $limit: 10, $order: col DESC]",
    "@fields": { "output_key": "column_or_expression" },
    "child[]": { "@source": "other_table[...]", "@fields": {...} }
  }
}
```

**Output JSON**:
```json
{ "isOk": true, "sql": "SELECT ...", "params": { "p1": "value" }, "message": "success" }
```

`uaq_free_string()` must be called on the returned C string to avoid memory leaks.
