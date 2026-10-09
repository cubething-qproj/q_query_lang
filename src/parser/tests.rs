use super::*;

/// The design doc's positive corpus, plus queries that parse but are
/// rejected later.
const POSITIVE: &[&str] = &[
    "Name",
    "@",
    "(Name Transform)",
    "(Name ?Health)",
    "@[Brush]",
    "Name[Enemy !Dead]",
    "Name[Enemy][!Dead]",
    "Shell[TerminalA or TerminalB]",
    "T[A B or C]",
    "T[(A B) or C]",
    "T[!(A B)]",
    "T[!!A]",
    "T[#West]",
    r#"T[!#"Floor West"]"#,
    "MeshMaterial3d<StandardMaterial>[Visible]",
    "Foo<A B<C>>",
    "my_crate::Foo",
    "my_crate::Foo<other::Bar>[my_crate::Baz]",
    "@12v3",
    "@(12v3 4v1)",
    "@12v3[Enemy] | Name",
    "#Floor",
    r#"#"Floor West""#,
    "#Floor | Children",
    "#Floor | Children[..]",
    "#Floor | Children[..][#West] | Brush",
    "#Floor | Children[2]",
    "#Floor | Children[1..3]",
    "#Floor | Children[1..]",
    "#Floor | Children[..3]",
    "#Floor | Children[1..=2]",
    "#Floor | Children[..=2]",
    "#Floor | Children[Foo]",
    "#Floor | Children[2][Foo]",
    "#West | ChildOf[0] | Name",
    "Foo[Bar] | @",
    "( Name   ?Health ) [ Enemy  !Dead ]",
    // Rejected at planning or execution, but syntactically valid.
    "Fooo",
    "Vec3",
    "Foo",
    "?Entity",
    "Name[Entity]",
    "Transform[..]",
    "(Name Children)[0]",
    "#Floor[..]",
    "Children[..][0]",
    "Name[0]",
    "Name | @12v3",
    "(Name Name)",
];

/// The design doc's negative corpus.
const NEGATIVE: &[&str] = &[
    "[Foo]",
    "?@",
    "??Foo",
    "@Foo",
    "@()",
    "#",
    "Foo[",
    "Foo[]",
    "Foo[]]",
    "Foo[1..2..3]",
    "Foo[..=]",
    "Foo[1..=]",
    "Foo[-1]",
    "Foo[or B]",
    "Foo[A or]",
    "Foo[!]",
    "Foo |",
    "| Foo",
    "()",
    "my_crate::",
    "::Foo",
    "Foo<>",
    // `or` is reserved inside filters, even as a path prefix.
    "T[or::Foo]",
];

#[test]
fn accepts_positive_corpus() {
    for input in POSITIVE {
        let program = parse(input);
        println!("> {input}\n{program:#?}\n");
        assert!(program.is_ok(), "{input:?}: {program:?}");
    }
}

#[test]
fn rejects_negative_corpus() {
    for input in NEGATIVE {
        let program = parse(input);
        match &program {
            Err(error) => println!("> {input}\nerror: {error}\n"),
            Ok(_) => panic!("{input:?} parsed: {program:?}"),
        }
    }
}

/// Parses a single-stage query and returns its stage.
fn stage(input: &str) -> Stage {
    let mut program = parse(input).unwrap();
    assert_eq!(program.stages.len(), 1, "{input:?}");
    program.stages.remove(0)
}

/// Parses `T[…]` and returns the filter of its only select.
fn select(input: &str) -> FilterKind {
    match stage(input).postfix.as_slice() {
        [Postfix::Select { filter, .. }] => filter.kind.clone(),
        other => panic!("{input:?}: {other:?}"),
    }
}

/// The type names of a filter's operands, for shape assertions.
fn shape(kind: &FilterKind) -> String {
    let join = |operands: &[Filter], op: &str| {
        let inner: Vec<_> = operands.iter().map(|f| shape(&f.kind)).collect();
        format!("({})", inner.join(op))
    };
    match kind {
        FilterKind::Has(ty) => ty.path.join("::"),
        FilterKind::Name(name) => format!("#{}", name.value),
        FilterKind::Not(operand) => format!("!{}", shape(&operand.kind)),
        FilterKind::And(operands) => join(operands, " and "),
        FilterKind::Or(operands) => join(operands, " or "),
    }
}

#[test]
fn or_binds_tighter_than_juxtaposition() {
    assert_eq!(shape(&select("T[A B or C]")), "(A and (B or C))");
    assert_eq!(shape(&select("T[A or B C]")), "((A or B) and C)");
    assert_eq!(shape(&select("T[(A B) or C]")), "((A and B) or C)");
    assert_eq!(shape(&select("T[A or B or C or D]")), "(A or B or C or D)");
}

#[test]
fn negation_binds_to_one_atom() {
    assert_eq!(shape(&select("T[!A or B]")), "(!A or B)");
    assert_eq!(shape(&select("T[!(A B)]")), "!(A and B)");
    assert_eq!(shape(&select(r#"T[!#"Floor West"]"#)), "!#Floor West");
}

#[test]
fn or_is_a_keyword_only_as_a_whole_word() {
    assert_eq!(shape(&select("T[A orB]")), "(A and orB)");
    assert_eq!(shape(&select("T[my_crate::or]")), "my_crate::or");
    assert!(matches!(
        stage("or").primary,
        Primary::Data(Data::Term(Term::Component { .. }))
    ));
}

#[test]
fn parses_paths_and_generics() {
    let Primary::Data(Data::Term(Term::Component { optional, ty, .. })) =
        stage("?a::B<c::D E<F>>").primary
    else {
        panic!("expected a component term");
    };
    assert!(optional);
    assert_eq!(ty.path, ["a", "B"]);
    assert_eq!(ty.args[0].path, ["c", "D"]);
    assert_eq!(ty.args[1].path, ["E"]);
    assert_eq!(ty.args[1].args[0].path, ["F"]);
}

#[test]
fn parses_entity_literals() {
    let Primary::Literal { ids, .. } = stage("@(12v3 4v1)").primary else {
        panic!("expected a literal");
    };
    let ids: Vec<_> = ids.iter().map(|id| (id.index, id.generation)).collect();
    assert_eq!(ids, [(12, 3), (4, 1)]);
    assert!(matches!(
        stage("@").primary,
        Primary::Data(Data::Term(Term::Entity(_)))
    ));
}

#[test]
fn distinguishes_takes_from_selects() {
    let range = |input: &str| match stage(input).postfix.as_slice() {
        [Postfix::Take { range, .. }] => *range,
        other => panic!("{input:?}: {other:?}"),
    };
    let half_open = |start, end| TakeRange::Range {
        start,
        end,
        inclusive: false,
    };
    assert_eq!(range("C[2]"), TakeRange::Index(2));
    assert_eq!(range("C[..]"), half_open(None, None));
    assert_eq!(range("C[1..]"), half_open(Some(1), None));
    assert_eq!(range("C[..3]"), half_open(None, Some(3)));
    assert_eq!(range("C[1..3]"), half_open(Some(1), Some(3)));
    assert_eq!(
        range("C[1..=3]"),
        TakeRange::Range {
            start: Some(1),
            end: Some(3),
            inclusive: true,
        }
    );
    assert!(matches!(
        stage("C[2][Foo]").postfix.as_slice(),
        [Postfix::Take { .. }, Postfix::Select { .. }]
    ));
}

#[test]
fn parses_pipelines_and_names() {
    let program = parse(r#"#"Floor West" | Children[..] | Brush"#).unwrap();
    assert_eq!(program.stages.len(), 3);
    let Primary::Name(name) = &program.stages[0].primary else {
        panic!("expected a name selector");
    };
    assert_eq!(name.value, "Floor West");
    assert!(name.quoted);
}

#[test]
fn spans_exclude_surrounding_whitespace() {
    let input = "  Name [ Enemy ]  ";
    let stage = stage(input);
    assert_eq!(&input[stage.span.clone()], "Name [ Enemy ]");
    let [Postfix::Select { filter, span }] = stage.postfix.as_slice() else {
        panic!("expected a select");
    };
    assert_eq!(&input[span.clone()], "[ Enemy ]");
    assert_eq!(&input[filter.span.clone()], "Enemy");
}

#[test]
fn reports_the_innermost_expectation_and_offset() {
    let cases = [
        ("Foo[A or]", Expected::Type, 8),
        ("Foo[or B]", Expected::Type, 4),
        ("T[A or::Foo]", Expected::Type, 6),
        ("Foo | ", Expected::Type, 6),
        ("my_crate::", Expected::Type, 10),
        ("Foo[A", Expected::CloseBracket, 5),
        ("Foo[1..=]", Expected::Integer, 8),
        ("C[99999999999999999999999]", Expected::Integer, 2),
        ("@(12v3 Name)", Expected::CloseParen, 7),
        ("@4294967296v1", Expected::EntityId, 1),
        ("#", Expected::Name, 1),
        ("Foo Bar", Expected::EndOfStage, 4),
    ];
    for (input, expected, offset) in cases {
        assert_eq!(
            parse(input),
            Err(ParseError { expected, offset }),
            "{input:?}"
        );
    }
}
