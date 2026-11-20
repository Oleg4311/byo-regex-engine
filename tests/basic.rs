//! Таблица примеров с ожидаемым поведением. Каждый пример дополнительно сверяется с крейтом `regex`,
//! поэтому ошибка в ожиданиях выявляется сразу, а не «закрепляется» в тестах.

use byo_regex::{Engine, ErrorKind, Limits, Regex, Stats};

type Spans = Vec<Option<(usize, usize)>>;

/// Крейт regex не считает хвостовые группы, недостижимые из-за `{0}`: обрезаем пустой хвост у обеих сторон.
fn trim(mut v: Spans) -> Spans {
    while v.len() > 1 && v.last() == Some(&None) {
        v.pop();
    }
    v
}

fn ours(re: &str, hay: &str) -> Option<Spans> {
    let re = Regex::new(re).unwrap();
    re.captures(hay).map(|c| {
        trim(
            (0..c.len())
                .map(|i| c.get(i).map(|m| (m.start(), m.end())))
                .collect(),
        )
    })
}

fn theirs(re: &str, hay: &str) -> Option<Spans> {
    let re = regex::Regex::new(re).unwrap();
    re.captures(hay).map(|c| {
        trim(
            (0..c.len())
                .map(|i| c.get(i).map(|m| (m.start(), m.end())))
                .collect(),
        )
    })
}

#[test]
fn table_of_examples() {
    // (шаблон, текст, ожидаемая вся совпавшая подстрока)
    let cases: &[(&str, &str, Option<&str>)] = &[
        // литералы и точка
        ("abc", "xxabcxx", Some("abc")),
        ("a.c", "a\nc abc", Some("abc")), // точка не берёт перевод строки
        ("", "abc", Some("")),
        // альтернатива: первый слева и первый по порядку, а не самый длинный
        ("a|ab", "ab", Some("a")),
        ("ab|a", "ab", Some("ab")),
        ("(a|ab)(c|bcd)", "abcd", Some("abcd")),
        // жадность и лень
        ("<.+>", "<a><b>", Some("<a><b>")),
        ("<.+?>", "<a><b>", Some("<a>")),
        ("a*?", "aaa", Some("")),
        ("a+?", "aaa", Some("a")),
        ("a??b", "ab", Some("ab")),
        // счётные повторы
        ("a{3}", "aaaaa", Some("aaa")),
        ("a{2,}", "aaaaa", Some("aaaaa")),
        ("a{2,3}", "aaaaa", Some("aaa")),
        ("a{2,3}?", "aaaaa", Some("aa")),
        ("(ab){2}", "ababab", Some("abab")),
        ("a{0}b", "aab", Some("b")),
        // классы
        ("[a-c]+", "xxabcabcxx", Some("abcabc")),
        ("[^a-c]+", "abxyzc", Some("xyz")),
        ("[]a]+", "x]a]y", Some("]a]")),
        ("[a-]+", "x-a-y", Some("-a-")),
        (r"[\d.]+", "ver 3.14.15!", Some("3.14.15")),
        (r"\d+", "abc 12345 def", Some("12345")),
        (r"\w+", "  hello_world1 ", Some("hello_world1")),
        (r"\s+", "a \t\n b", Some(" \t\n ")),
        (r"\D+", "12ab34", Some("ab")),
        (r"[\W]+", "ab, cd", Some(", ")),
        // якоря и границы
        ("^abc", "abcabc", Some("abc")),
        ("^abc", "xabc", None),
        ("abc$", "abcabc", Some("abc")),
        ("abc$", "abcx", None),
        ("^$", "", Some("")),
        (r"\bcat\b", "concat cat cats", Some("cat")),
        (r"\Bcat", "cat concat", Some("cat")),
        // вложенность и пустые ветви
        ("(a|)+b", "aab", Some("aab")),
        ("(|a)+b", "aab", Some("aab")),
        ("x(y(z)?)*", "xyzyy", Some("xyzyy")),
        // экранирование
        (r"a\.b", "a.b axb", Some("a.b")),
        (r"\x41\x42", "zAB", Some("AB")),
        (r"\(\)", "f()", Some("()")),
        // юникод: символ — это один Unicode-скаляр, а не байт
        ("п.ивет", "привет", Some("привет")),
        ("[а-я]+", "Привет мир", Some("ривет")),
        ("^.$", "ж", Some("ж")),
        ("(?:ы|ъ)+", "абыъыв", Some("ыъы")),
    ];
    for (re, hay, want) in cases {
        let got = Regex::new(re)
            .unwrap()
            .find(hay)
            .map(|m| m.as_str().to_string());
        assert_eq!(got.as_deref(), *want, "наш движок: /{re}/ на {hay:?}");
        let reference = regex::Regex::new(re)
            .unwrap()
            .find(hay)
            .map(|m| m.as_str().to_string());
        assert_eq!(
            reference.as_deref(),
            *want,
            "ожидание расходится с крейтом regex: /{re}/ на {hay:?}"
        );
    }
}

#[test]
fn captures_match_the_reference_implementation() {
    let cases: &[(&str, &str)] = &[
        (r"(\d+)-(\d+)", "tel 123-4567"),
        (r"(a)|(b)", "b"),
        (r"(a)?(b)", "b"),
        (r"(a*)+", "aaa"),
        (r"(a*)*", "b"),
        (r"(a|b)*c", "abbac"),
        (r"((a)|(b))+", "ab"),
        (r"(?P<y>\d{4})-(?P<m>\d{2})", "x 2026-11 y"),
        (r"(?:(\w)\1)?", "aa"), // обратные ссылки не поддерживаются: проверим отдельно
    ];
    for (re, hay) in cases {
        if re.contains(r"\1") {
            assert!(
                Regex::new(re).is_err(),
                "обратная ссылка должна отвергаться: {re}"
            );
            continue;
        }
        assert_eq!(ours(re, hay), theirs(re, hay), "/{re}/ на {hay:?}");
    }
}

#[test]
fn named_groups_and_indexing() {
    let re = Regex::new(r"(?P<key>\w+)=(?<val>[^;]*);?").unwrap();
    let caps = re.captures("name=Анна; age=30").unwrap();
    assert_eq!(caps.name("key").unwrap().as_str(), "name");
    assert_eq!(caps.name("val").unwrap().as_str(), "Анна");
    assert_eq!(&caps[0], "name=Анна;");
    assert_eq!(caps.len(), 3);
    assert!(caps.name("nope").is_none());
}

#[test]
fn iteration_matches_reference() {
    let cases: &[(&str, &str)] = &[
        ("a*", "baaac"),
        ("a*?", "baaac"),
        ("", "абв"),
        (r"\b", "ab cd"),
        (r"\d+", "1 22 333"),
        ("x*", ""),
        (r"(?:a|b)*", "abcab"),
        ("^", "aaa"),
        ("$", "aaa"),
    ];
    for (re, hay) in cases {
        let a: Vec<_> = Regex::new(re)
            .unwrap()
            .find_iter(hay)
            .map(|m| m.range())
            .collect();
        let b: Vec<_> = regex::Regex::new(re)
            .unwrap()
            .find_iter(hay)
            .map(|m| m.range())
            .collect();
        assert_eq!(a, b, "find_iter /{re}/ на {hay:?}");
    }
}

#[test]
fn stacked_quantifiers_and_quantified_assertions_follow_the_reference() {
    for re in [
        "a**", "a+*", "a{2}{3}", "a*+", "^*", "$+", r"\b?", "(a*)*", "(a*)+", "(a|b*)*c",
        "(?:a?)*b",
    ] {
        let (o, r) = (Regex::new(re), regex::Regex::new(re));
        assert_eq!(o.is_ok(), r.is_ok(), "принятие /{re}/");
        for hay in ["", "b", "aab", "ababc", "aaaa"] {
            assert_eq!(ours(re, hay), theirs(re, hay), "/{re}/ на {hay:?}");
        }
    }
}

#[test]
fn invalid_patterns_are_rejected_with_position() {
    let bad: &[(&str, ErrorKind)] = &[
        ("(", ErrorKind::UnmatchedOpenParen),
        ("a)", ErrorKind::UnmatchedCloseParen),
        ("*a", ErrorKind::NothingToRepeat),
        ("+", ErrorKind::NothingToRepeat),
        ("a|*", ErrorKind::NothingToRepeat),
    ];
    for (re, kind) in bad {
        let e = Regex::new(re).expect_err(re);
        assert_eq!(&e.kind, kind, "/{re}/: {e}");
        assert!(
            regex::Regex::new(re).is_err(),
            "крейт regex принял бы /{re}/: ожидание теста неверно"
        );
    }
    for re in [
        "[",
        "[a",
        "[z-a]",
        "a{2,1}",
        "a{",
        r"\q",
        "(?=a)",
        "(?i)a",
        "[[:alpha:]]",
        r"(\w)\1",
        "a{1001}",
        "(?P<n>a)(?P<n>b)",
        r"\x4",
        "[\\d-z]",
    ] {
        assert!(Regex::new(re).is_err(), "/{re}/ должен отвергаться");
    }
    let e = Regex::new("ab(cd").unwrap_err();
    assert_eq!(e.pos, 2, "позиция должна указывать на непарную скобку");
}

#[test]
fn resource_limits_protect_against_hostile_patterns() {
    // глубокая вложенность не должна переполнять стек
    let deep = format!("{}a{}", "(".repeat(10_000), ")".repeat(10_000));
    assert_eq!(Regex::new(&deep).unwrap_err().kind, ErrorKind::TooDeep);
    let flat = format!("{}a{}", "(".repeat(200), ")".repeat(200));
    assert!(Regex::new(&flat).is_ok());
    // взрыв при раскрытии счётных повторов
    assert_eq!(
        Regex::new("((a{1000}){1000}){1000}").unwrap_err().kind,
        ErrorKind::TooBig
    );
    assert_eq!(
        Regex::new("(a{1000}){1000}").unwrap_err().kind,
        ErrorKind::TooBig
    );
    // разумный большой повтор работает
    let re = Regex::new("a{500}").unwrap();
    assert!(re.is_match(&"a".repeat(500)));
    assert!(!re.is_match(&"a".repeat(499)));
}

#[test]
fn pike_is_linear_on_catastrophic_patterns_and_backtracking_is_not() {
    let hay = format!("{}c", "a".repeat(30));
    let re = Regex::new("(a+)+b").unwrap();

    let mut pike = Stats::default();
    assert!(re
        .captures_with(&hay, Engine::Pike, &mut pike)
        .unwrap()
        .is_none());
    assert!(
        pike.steps < 5_000,
        "Pike должен работать за линейное число шагов, а сделал {}",
        pike.steps
    );

    let mut bt = Stats::default();
    let limits = Limits {
        max_steps: 2_000_000,
        memoize: false,
    };
    assert!(
        re.captures_with(&hay, Engine::Backtrack(limits), &mut bt)
            .is_err(),
        "классический откат должен упереться в предел шагов"
    );

    // запоминание пар (инструкция, позиция) возвращает откату полиномиальное время и тот же ответ
    let mut memo = Stats::default();
    let memo_limits = Limits {
        max_steps: 2_000_000,
        memoize: true,
    };
    assert!(re
        .captures_with(&hay, Engine::Backtrack(memo_limits), &mut memo)
        .unwrap()
        .is_none());
    assert!(
        memo.steps < 20_000,
        "откат с запоминанием: {} шагов",
        memo.steps
    );
}

#[test]
fn pike_steps_grow_linearly_with_input_length() {
    let re = Regex::new("(a|aa)+b").unwrap();
    let steps = |n: usize| {
        let mut s = Stats::default();
        assert!(re
            .captures_with(&"a".repeat(n), Engine::Pike, &mut s)
            .unwrap()
            .is_none());
        s.steps
    };
    let (s1, s2) = (steps(1_000), steps(2_000));
    let ratio = s2 as f64 / s1 as f64;
    assert!(
        (1.8..2.3).contains(&ratio),
        "при удвоении текста шагов должно быть вдвое больше, а отношение {ratio:.2}"
    );
}

#[test]
fn long_inputs_do_not_overflow_the_stack() {
    let hay = "ab".repeat(200_000);
    let re = Regex::new("(?:a|b)*c").unwrap();
    assert!(!re.is_match(&hay));
    let re = Regex::new("(ab)*$").unwrap();
    assert_eq!(re.find(&hay).unwrap().range(), 0..hay.len());
}

#[test]
fn ascii_semantics_of_word_classes_is_documented_behaviour() {
    // \w и \b у нас ASCII (в отличие от крейта regex, где они Unicode): это ограничение описано в README
    let re = Regex::new(r"\w+").unwrap();
    assert_eq!(re.find("жук bug").unwrap().as_str(), "bug");
    assert!(Regex::new(r"\W").unwrap().is_match("é"));
}

#[test]
fn start_offset_keeps_context_for_anchors_and_boundaries() {
    let re = Regex::new(r"^a").unwrap();
    assert!(
        re.captures_at("aa", 1).is_none(),
        "^ — это начало всего текста, а не позиции поиска"
    );
    let re = Regex::new(r"\bb").unwrap();
    assert!(
        re.captures_at("ab", 1).is_none(),
        "между a и b нет границы слова"
    );
    assert!(
        Regex::new("a").unwrap().captures_at("éa", 1).is_none(),
        "позиция внутри символа — не граница, поиска нет"
    );
}

/// Известное и осознанное различие: классический откат (семантика Perl) принимает пустую итерацию цикла,
/// а Pike VM и крейт regex — нет, потому что в (инструкцию, позицию) побеждает первый пришедший.
#[test]
fn classic_backtracking_differs_from_pike_only_in_degenerate_empty_loops() {
    let re = Regex::new("(.{0,1})*").unwrap();
    let span = |engine| {
        let mut st = Stats::default();
        let c = re.captures_with("a", engine, &mut st).unwrap().unwrap();
        (c.get(0).map(|m| m.range()), c.get(1).map(|m| m.range()))
    };
    let classic = Engine::Backtrack(Limits {
        max_steps: 100_000,
        memoize: false,
    });
    let memo = Engine::Backtrack(Limits {
        max_steps: 100_000,
        memoize: true,
    });
    assert_eq!(span(Engine::Pike), (Some(0..1), Some(0..1)));
    assert_eq!(
        span(memo),
        span(Engine::Pike),
        "откат с запоминанием обязан совпадать с Pike"
    );
    assert_eq!(
        span(classic),
        (Some(0..1), Some(1..1)),
        "классический откат берёт пустую итерацию (как Perl)"
    );
    // и крейт regex на стороне Pike
    let c = regex::Regex::new("(.{0,1})*")
        .unwrap()
        .captures("a")
        .unwrap();
    assert_eq!(c.get(1).map(|m| m.range()), Some(0..1));
}
