# q_query_lang

[![Coverage Status](https://coveralls.io/repos/github/cubething-qproj/q_query_lang/badge.svg)](https://coveralls.io/github/cubething-qproj/q_query_lang)

A powerful string-based query language for Bevy. Programs are made from
pipelines of queries. Queries have the form, `QueryData[QueryFilter]`, with
special syntax for entity access, relationship traversal, and name filtering,
among others. Each stage of the pipeline runs over the source entities of the
previous stage's queried rows.

## What this is, and what it isn't

This is the language parser and executor. It does not offer a full scripting
utility, nor a command line for interactive use. See
[q_term](https://github.com/cubething-qproj/q_term),
[q_shell](https://github.com/cubething-qproj/q_shell), and
[q_proc](https://github.com/cubething-qroj/q_proc) for integrations.

## Examples

This first query reads each enemy's name and health:

```
(Name ?Health)[Enemy !Dead]
```

```rust
fn system(q: Query<(&Name, Option<&Health>), (With<Enemy>, Without<Dead>)>) {
    let rows: Vec<_> = q.iter().collect();
}
```

This one follows a jackdaw brush object named `Floor`, traverses its `Children`
to the child named `West`, and reads its `Brush` component.

```
#Floor | Children[..][#West] | Brush
```
```rust
fn system(
    names: Query<(Entity, &Name)>,
    children: Query<&Children>,
    brushes: Query<&Brush>,
) {
    let rows: Vec<_> = names
        .iter()
        .filter(|(_, n)| n.as_str() == "Floor")
        .filter_map(|(e, _)| children.get(e).ok())
        .flat_map(|c| c.iter())
        .filter(|&c| names.get(c).is_ok_and(|(_, n)| n.as_str() == "West"))
        .filter_map(|c| brushes.get(c).ok())
        .collect();
}
```

The third reads two components from two specific entities:

```
@(12v3 4v1) | (Name Transform)
```
```rust
fn system(In(entities): In<[Entity; 2]>, q: Query<(&Name, &Transform)>) {
    let rows: Vec<_> = entities.into_iter().filter_map(|e| q.get(e).ok()).collect();
}
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE.txt) or
[MIT License](LICENSE-MIT.txt) at your option.
