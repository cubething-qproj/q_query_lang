//! A read-only, string-based query language for Bevy.
//!
//! A query is a pipeline of stages joined by `|`. Each stage is a Bevy-style
//! `D[F]` query: a data clause `D`, then postfix operators in brackets. The
//! first stage runs over the whole World; every later stage runs over the
//! **source entities** of the previous stage's rows.
//!
//! The normative grammar and semantics live in the org-level
//! `q_query_lang design.md`.
//!
//! # Syntax at a glance
//!
//! | Query                           | Meaning                                                   |
//! | ------------------------------- | --------------------------------------------------------- |
//! | `Name`                          | `Query<&Name>`                                            |
//! | `(Name ?Health)`                | `Query<(&Name, Option<&Health>)>`                         |
//! | `@[Brush]`                      | `Query<Entity, With<Brush>>`                              |
//! | `Name[Enemy !Dead]`             | `Query<&Name, (With<Enemy>, Without<Dead>)>`              |
//! | `Shell[TerminalA or TerminalB]` | `Query<&Shell, Or<(With<TerminalA>, With<TerminalB>)>>`   |
//! | `my_crate::Foo<Bar>`            | a path-qualified, generic component                       |
//! | `@12v3 \| (Name Transform)`     | `query.get(e)` on one entity                              |
//! | `@(12v3 4v1) \| Transform`      | `query.get_many([a, b])`                                  |
//! | `#Floor`                        | every entity named `Floor`                                |
//! | `#Floor \| Children[..]`        | Floor's children                                          |
//! | `#Floor \| Children[1..=2]`     | Floor's second and third children                         |
//! | `#West \| ChildOf[0] \| Name`   | West's parent's `Name`                                    |
//!
//! - `@` is the entity id (`Entity`); `?T` is `Option<&T>`.
//! - In brackets, an integer or range is a **take** (follow a relationship);
//!   anything else is a **select** (a filter).
//! - In filters, `T` is `With<T>`, `!T` is `Without<T>`, `#n` tests `Name`,
//!   juxtaposition is AND, and `or` is OR.
//! - Whitespace may appear between any two tokens.
//!
//! # Gotchas
//!
//! - **`#n` yields entities, not names.** `#Floor` is sugar for
//!   `@[#Floor]`. To read the `Name` itself, query it: `Name[#Floor]`. Names
//!   are not unique, so `#Floor` may match any number of entities. Names that
//!   are not identifiers need quotes: `#"Floor West"`.
//! - **`or` binds tighter than juxtaposition.** `[A B or C]` is
//!   `A and (B or C)`. Group a conjunction explicitly: `[(A B) or C]`.
//! - **Fetching is not following.** `Children` is the `Children` component;
//!   `Children[..]` is the children.
//! - **A select tests the row's source entity.** `Children[Foo]` keeps the
//!   `Children` of entities that have `Foo`. To keep children that have
//!   `Foo`, take first: `Children[..][Foo]`.
//! - **Take is per row.** `@[Brush] | Children[0]` is the first child of
//!   _each_ brush, not the first child overall. There is no positional
//!   selection over a stream: `Name[0]` is an error.
//! - **Stages connect only through source entities.** `#e | Health | Name`
//!   is legal: it reads the `Name` of every entity named `e` that has
//!   `Health`.
//! - **Entity literals only start a query.** `Name | @12v3` is an error.
//! - **`or` is reserved inside filters.** A type named `or` must be
//!   path-qualified there: `[my_crate::or]`.

pub mod ast;
mod parser;

pub use parser::{Expected, ParseError, parse};
