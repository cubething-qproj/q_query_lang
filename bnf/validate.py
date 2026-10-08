#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["abnf[rust]"]
# ///
"""Validate the normative q_query_lang ABNF grammar against its corpus."""

import sys
from pathlib import Path

from abnf import ParseError, Rule  # type: ignore[import-not-found]

GRAMMAR_FILE = Path(__file__).with_name("grammar.ebnf")

POSITIVE = """\
#Floor/Children::*/Brush
#Floor/Children::*/Brush.faces[1].material
#Floor/Children::#"Floor West"/Brush
#Floor/Children::*[1]/Brush
#Floor/Children
#Floor/ChildOf::*
#Floor/..
Name[+Enemy -Dead]
Name[+Enemy][-Dead]
Name[+Enemy and -Dead]
Name[with(Enemy)]
Name[not(without(Dead))]
Name[(+Enemy or -Dead)]
^Transform[changed(Velocity)]
Shell[+TerminalA or +TerminalB]
Health[added(Enemy)]
Transform[spawned()]
@[+Brush]
#enemy/Health
#enemy/?Health
#enemy/?^Health
#enemy/@
#enemy
@
Entity
#enemy/Health.hp
Health|Armor
#Terminal/shell.0/Process
#a/a.0/b.0/c
//*[+Brush][1]
//Foo[+Bar]
/#Floor
#Floor//*
..
Foo
optional(Foo)
mut(Health)
optional(mut(Health))
ref(Health)
single(#Floor)
single(Name[+Enemy])
#enemy/@/Health
Brush.faces
Brush.faces[last()]
#Ramps/Children::#Floor
""".splitlines()

NEGATIVE = """\
[+Brush]
@Brush
Foo.
#
?@
^@
//
#Floor//
Foo[+Bar
+Enemy
Children::
Children::Brush
boolean(Player)
""".splitlines()


def main() -> int:
    grammar_text = GRAMMAR_FILE.read_text(encoding="ascii")

    class QueryGrammar(Rule):
        pass

    QueryGrammar.load_grammar(grammar_text)
    query = QueryGrammar("query")

    failures = 0
    for sentence in POSITIVE:
        try:
            query.parse_all(sentence)
        except ParseError as exc:
            failures += 1
            print(f"FAIL (expected parse): {sentence!r} at {exc.start}")
        else:
            print(f"ok   {sentence}")

    for sentence in NEGATIVE:
        try:
            query.parse_all(sentence)
        except ParseError:
            print(f"ok   (rejected) {sentence}")
        else:
            failures += 1
            print(f"FAIL (expected reject): {sentence!r}")

    total = len(POSITIVE) + len(NEGATIVE)
    print(f"\n{total - failures}/{total} passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
