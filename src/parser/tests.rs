use super::*;

const POSITIVE: &[&str] = &[
    "#Floor/Children::*/Brush",
    "#Floor/Children::*/Brush.faces[1].material",
    "#Floor/Children::#\"Floor West\"/Brush",
    "#Floor/Children::*[1]/Brush",
    "#Floor/Children",
    "#Floor/ChildOf::*",
    "#Floor/..",
    "Name[+Enemy -Dead]",
    "Name[+Enemy][-Dead]",
    "Name[+Enemy and -Dead]",
    "Name[with(Enemy)]",
    "Name[not(without(Dead))]",
    "Name[(+Enemy or -Dead)]",
    "^Transform[changed(Velocity)]",
    "Shell[+TerminalA or +TerminalB]",
    "Health[added(Enemy)]",
    "Transform[spawned()]",
    "@[+Brush]",
    "#enemy/Health",
    "#enemy/?Health",
    "#enemy/?^Health",
    "#enemy/@",
    "#enemy",
    "@",
    "Entity",
    "#enemy/Health.hp",
    "Health|Armor",
    "#Terminal/shell.0/Process",
    "#a/a.0/b.0/c",
    "//*[+Brush][1]",
    "//Foo[+Bar]",
    "/#Floor",
    "#Floor//*",
    "..",
    "Foo",
    "optional(Foo)",
    "mut(Health)",
    "optional(mut(Health))",
    "ref(Health)",
    "single(#Floor)",
    "single(Name[+Enemy])",
    "#enemy/@/Health",
    "Brush.faces",
    "Brush.faces[last()]",
    "#Ramps/Children::#Floor",
];

const NEGATIVE: &[&str] = &[
    "[+Brush]",
    "@Brush",
    "Foo.",
    "#",
    "?@",
    "^@",
    "//",
    "#Floor//",
    "Foo[+Bar",
    "+Enemy",
    "Children::",
    "Children::Brush",
    "boolean(Player)",
];

#[test]
fn accepts_normative_corpus() {
    for input in POSITIVE {
        assert!(
            parse(input).is_ok(),
            "expected {input:?} to parse: {:?}",
            parse(input)
        );
    }
}

#[test]
fn rejects_normative_corpus() {
    for input in NEGATIVE {
        assert!(parse(input).is_err(), "expected {input:?} to be rejected");
    }
}

#[test]
fn uses_path_separators_for_leading_and_internal_positions() {
    for (input, expected) in [
        ("/#Floor", Some(PathSeparator::Slash)),
        ("//Foo", Some(PathSeparator::Descendants)),
        ("Foo", None),
    ] {
        let query = parse(input).unwrap();
        let TermAst::Path(path) = &query.terms[0] else {
            panic!("expected path")
        };
        assert_eq!(path.prefix, expected, "{input:?}");
    }
}

#[test]
fn parses_complete_prefixed_filter_ast() {
    assert_eq!(
        parse("//Foo[+Bar]").unwrap(),
        QueryAst {
            terms: vec![TermAst::Path(PathAst {
                prefix: Some(PathSeparator::Descendants),
                first: StepAst::Fetch(FetchStepAst {
                    fetch: FetchAst::Component {
                        access: AccessMode::Read,
                        ty: TypeName {
                            value: "Foo",
                            span: 2..5,
                        },
                        span: 2..5,
                    },
                    predicates: vec![PredicateAst {
                        kind: PredicateKind::With(TypeName {
                            value: "Bar",
                            span: 7..10,
                        }),
                        span: 6..10,
                    }],
                    fields: vec![],
                    span: 2..11,
                }),
                rest: vec![],
                span: 0..11,
            })],
            span: 0..11,
        }
    );
}

#[test]
fn parses_complete_single_union_ast() {
    assert_eq!(
        parse("single(@)|Entity").unwrap(),
        QueryAst {
            terms: vec![
                TermAst::Single {
                    query: Box::new(QueryAst {
                        terms: vec![TermAst::Path(PathAst {
                            prefix: None,
                            first: StepAst::Fetch(FetchStepAst {
                                fetch: FetchAst::Entity { span: 7..8 },
                                predicates: vec![],
                                fields: vec![],
                                span: 7..8,
                            }),
                            rest: vec![],
                            span: 7..8,
                        })],
                        span: 7..8,
                    }),
                    span: 0..9,
                },
                TermAst::Path(PathAst {
                    prefix: None,
                    first: StepAst::Fetch(FetchStepAst {
                        fetch: FetchAst::Component {
                            access: AccessMode::Read,
                            ty: TypeName {
                                value: "Entity",
                                span: 10..16,
                            },
                            span: 10..16,
                        },
                        predicates: vec![],
                        fields: vec![],
                        span: 10..16,
                    }),
                    rest: vec![],
                    span: 10..16,
                }),
            ],
            span: 0..16,
        }
    );
}

#[test]
fn parses_fetch_with_conjunctive_filters() {
    let query = parse("Foo[+Bar -Baz]").unwrap();
    let TermAst::Path(path) = &query.terms[0] else {
        panic!("expected path")
    };
    let StepAst::Fetch(fetch) = &path.first else {
        panic!("expected fetch")
    };
    let FetchAst::Component { access, ty, .. } = &fetch.fetch else {
        panic!("expected component")
    };
    assert_eq!(*access, AccessMode::Read);
    assert_eq!(ty.value, "Foo");
    assert!(matches!(fetch.predicates[0].kind, PredicateKind::And(_)));
}

#[test]
fn parses_relationship_axis_and_fields() {
    let query = parse("#Floor/Children::#West/Brush.faces[1].material").unwrap();
    let TermAst::Path(path) = &query.terms[0] else {
        panic!("expected path")
    };
    let StepAst::Node(axis) = &path.rest[0].1 else {
        panic!("expected relationship axis")
    };
    assert_eq!(axis.relationship.as_ref().unwrap().value, "Children");
    assert!(matches!(&axis.test, NodeTest::Name(name) if name.value == "West"));

    let StepAst::Fetch(fetch) = &path.rest[1].1 else {
        panic!("expected component fetch")
    };
    assert_eq!(fetch.fields.len(), 3);
    assert!(matches!(fetch.fields[0].kind, FieldKind::Named("faces")));
    assert!(matches!(fetch.fields[1].kind, FieldKind::Position(1)));
    assert!(matches!(fetch.fields[2].kind, FieldKind::Named("material")));
}

#[test]
fn parses_entity_reentry_chain() {
    let query = parse("#Terminal/shell.0/Process").unwrap();
    let TermAst::Path(path) = &query.terms[0] else {
        panic!("expected path")
    };
    let StepAst::Fetch(shell) = &path.rest[0].1 else {
        panic!("expected shell fetch")
    };
    assert!(matches!(shell.fields[0].kind, FieldKind::Tuple(0)));
    assert!(matches!(&path.rest[1].1, StepAst::Fetch(_)));
}

#[test]
fn parses_single_and_union() {
    let query = parse("single(Name[+Enemy])|@").unwrap();
    assert_eq!(query.terms.len(), 2);
    assert!(matches!(query.terms[0], TermAst::Single { .. }));
    assert!(matches!(query.terms[1], TermAst::Path(_)));
}

#[test]
fn fetch_spans_include_access_syntax() {
    for (input, expected_access) in [
        ("?Foo", AccessMode::OptionalRead),
        ("^Foo", AccessMode::Write),
        ("?^Foo", AccessMode::OptionalWrite),
        ("optional(Foo)", AccessMode::OptionalRead),
        ("optional(mut(Foo))", AccessMode::OptionalWrite),
        ("mut(Foo)", AccessMode::Write),
    ] {
        let query = parse(input).unwrap();
        let TermAst::Path(path) = &query.terms[0] else {
            panic!("expected path")
        };
        let StepAst::Fetch(fetch) = &path.first else {
            panic!("expected fetch")
        };
        let FetchAst::Component { access, span, .. } = &fetch.fetch else {
            panic!("expected component")
        };
        assert_eq!(*access, expected_access, "{input:?}");
        assert_eq!(span, &(0..input.len()), "{input:?}");
    }
}

#[test]
fn reports_error_offsets() {
    for (input, expected) in [
        ("Foo[+Bar", 8),
        ("Foo.", 4),
        ("Children::", 10),
        ("@Brush", 1),
    ] {
        assert_eq!(parse(input).unwrap_err().offset, expected, "{input:?}");
    }
}

#[test]
fn normative_grammar_is_present() {
    let grammar = include_str!("../../bnf/grammar.ebnf");
    assert!(grammar.contains("query"));
    assert!(grammar.contains("rel-axis"));
}
