---
name: q-query-lang
description: Deep dive on q_query_lang (lib/q_query_lang), the read-only string query language over a Bevy World — syntax (D[F] stages, pipelines with |, @ entity, ?Optional, #Name, takes like Children[..], filters with ! and or), QueryPlan planning against the TypeRegistry, execution to reflected snapshot rows, error types, and gotchas. Use when writing or debugging q queries, exposing queries in a shell/console, or changing the parser/planner/executor.
---

# q_query_lang

Source: `lib/q_query_lang/src/{lib.rs (syntax table + gotchas), ast.rs,
parser.rs (winnow), plan.rs, exec.rs}`. Normative grammar & semantics:
`.docs/designs/q_query_lang design.md` (org root). Tests: `src/parser/tests.rs`,
`tests/{plan,exec}.rs`. Depends on bevy with `default-features = false,
features = ["std"]`.

## API

```rust
use q_query_lang::QueryPlan;
let registry = world.resource::<AppTypeRegistry>().clone();
let plan = QueryPlan::new("(Name ?Health)[Enemy !Dead]", &registry.read())?; // no World access
plan.schema();   // Schema(Vec<Column { label, ty: Entity|Component|Optional }>)
plan.reads();    // component TypeIds read
let result = plan.execute(&mut world)?;      // needs &mut World (dynamic QueryState)
for row in &result.rows { row.source; row.value; } // RowValue::{Entity, Component(Box<dyn Reflect>), Absent, Tuple}
println!("{result}");                        // QueryResult: Display
```

Also public: `parse(&str) -> Result<ast::Program, ParseError>` and the
`ast` module (spanned, owned nodes).

**Types must be reflect-registered with `#[reflect(Component)]`**, or planning
fails (`UnknownType` / `NotAComponent`). Ambiguous short names →
`AmbiguousType` with candidates; path-qualify (`my_crate::Foo<Bar>`).
Opaque reflect types can be filtered on but not fetched in the final stage.

## Syntax

| Query | Bevy equivalent |
| --- | --- |
| `Name` | `Query<&Name>` |
| `(Name ?Health)` | `Query<(&Name, Option<&Health>)>` |
| `@[Brush]` | `Query<Entity, With<Brush>>` |
| `Name[Enemy !Dead]` | `Query<&Name, (With<Enemy>, Without<Dead>)>` |
| `Shell[A or B]` | `Or<(With<A>, With<B>)>` |
| `@12v3 \| (Name Transform)` | `query.get(e)` |
| `@(12v3 4v1) \| Transform` | `query.get_many([a, b])` |
| `#Floor` | entities named `Floor` (= `@[#Floor]`) |
| `#Floor \| Children[..]` | Floor's children |
| `#Floor \| Children[1..=2]` | 2nd and 3rd children |
| `#West \| ChildOf[0] \| Name` | West's parent's `Name` |

Each stage after the first runs over the **source entities** of the previous
stage's rows. Inside `[...]`, an integer/range is a **take** (follow a
relationship); anything else is a **select** (filter on the row's source).

## Gotchas (from `lib.rs`)

- `#n` yields entities; read the name with `Name[#Floor]`. Quote non-idents:
  `#"Floor West"`.
- `or` binds tighter than juxtaposition: `[A B or C]` = `A and (B or C)`.
- `Children` fetches the component; `Children[..]` follows it.
- `Children[Foo]` filters the *parent*; `Children[..][Foo]` filters children.
- Takes are per row; there's no positional selection over a stream
  (`Name[0]` is an error).
- Entity literals only start a query. Literals for despawned entities →
  `ExecError::StaleEntity`; ones hidden by default query filters (e.g.
  `Disabled`) → `FilteredEntity`.
- Takes on non-relationship components → `ExecError::NotARelationship`.
- In a shell, quote the whole query so it arrives as one argv string:
  `q '#Floor | Children[..] | (@ Name ?Health)'`.

## Integration

The only consumer today is `lib/q_shell/examples/query_shell.rs` (an
exclusive-system `q` program). It's explicitly a prototype; a real home will
come with qscript (`.docs/designs/q_shell_lang-design.md`).
