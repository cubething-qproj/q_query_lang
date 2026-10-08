# q_query_lang

An XPath-based query language for Bevy.

Queries read like paths, with Bevy `QueryData` first and filters attached in
brackets:

```text
Name[+Enemy -Dead]
#Floor/Children::*/Brush.faces[1].material
#Terminal/shell.0/Process
```

The first query corresponds to
`Query<&Name, (With<Enemy>, Without<Dead>)>`. The second follows Bevy's
`Children` relationship before projecting a reflected field. The third follows
an ordinary Entity-valued component field through re-entry.

See `bnf/grammar.ebnf` for the normative syntax.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE.txt) or
[MIT License](LICENSE-MIT.txt) at your option.
