# q_query_lang

A read-only, string-based query language for Bevy. A program is a pipeline of
Bevy-style `D[F]` queries; each stage runs over the entities the previous
stage produced.

```text
(Name ?Health)[Enemy !Dead]
#Floor | Children[..][#West] | Brush
@(12v3 4v1) | (Name Transform)
```

The first query corresponds to
`Query<(&Name, Option<&Health>), (With<Enemy>, Without<Dead>)>`. The second
follows Floor's `Children` to the child named West and reads its `Brush`. The
third reads two components from two specific entities.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE.txt) or
[MIT License](LICENSE-MIT.txt) at your option.
