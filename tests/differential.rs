//! Дифференциальное тестирование: на случайных шаблонах и текстах наш движок должен давать ровно тот же ответ,
//! что и крейт `regex` — границы совпадения, все группы захвата и итерацию по совпадениям.
//! Три наших движка (Pike, откат, откат с запоминанием) должны согласоваться между собой.
//!
//! Число случаев: переменная окружения PROPTEST_CASES (по умолчанию 256; в CI — 5000).

use byo_regex::ast::Node;
use byo_regex::parse::parse;
use byo_regex::{Engine, Limits, Regex, Stats};
use proptest::prelude::*;

type Spans = Vec<Option<(usize, usize)>>;

/// Шаблон строится из грамматики, а не из случайных символов: иначе почти всё отвергалось бы как ошибка синтаксиса.
fn atom() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        4 => prop::sample::select(vec!["a", "b", "c", "1", " ", "_"]).prop_map(String::from),
        1 => Just(".".to_string()),
        2 => prop::sample::select(vec!["[ab]", "[^a]", "[a-c]", "[^a-b1]", r"[\d_]", "[b-]", r"[\w\s]"]).prop_map(String::from),
        2 => prop::sample::select(vec![r"\d", r"\w", r"\s", r"\D", r"\W", r"\S"]).prop_map(String::from),
    ];
    leaf
}

fn quantifier() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => Just(String::new()),
        2 => Just("*".to_string()),
        2 => Just("+".to_string()),
        2 => Just("?".to_string()),
        1 => Just("*?".to_string()),
        1 => Just("+?".to_string()),
        1 => Just("??".to_string()),
        1 => (0u32..3, 0u32..3).prop_map(|(a, b)| format!("{{{},{}}}", a.min(b), a.max(b))),
        1 => (0u32..4).prop_map(|n| format!("{{{n}}}")),
        1 => (0u32..3).prop_map(|n| format!("{{{n},}}")),
    ]
}

fn assertion() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["^", "$", r"\b", r"\B"]).prop_map(String::from)
}

fn regex_strategy() -> impl Strategy<Value = String> {
    let item = (atom(), quantifier()).prop_map(|(a, q)| format!("{a}{q}"));
    let leaf = prop_oneof![8 => item, 1 => assertion(), 1 => (assertion(), quantifier()).prop_map(|(a, q)| format!("{a}{q}"))];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            3 => prop::collection::vec(inner.clone(), 1..4).prop_map(|v| v.concat()),
            2 => prop::collection::vec(inner.clone(), 2..4).prop_map(|v| v.join("|")),
            2 => (inner.clone(), quantifier()).prop_map(|(x, q)| format!("({x}){q}")),
            1 => (inner.clone(), quantifier()).prop_map(|(x, q)| format!("(?:{x}){q}")),
        ]
    })
}

fn haystack() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['a', 'b', 'c', '1', ' ', '_', '\n', '-']),
        0..14,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// Крейт regex не включает в captures_len хвостовые группы, недостижимые из-за `{0}`; нумерация остальных у нас та же.
/// Поэтому перед сравнением отбрасываем пустые группы в конце.
fn trim(mut v: Spans) -> Spans {
    while v.len() > 1 && v.last() == Some(&None) {
        v.pop();
    }
    v
}

/// Есть ли цикл (`*`, `+`, `{n,}`, `{n,m}` при m>1), тело которого может совпасть с пустой строкой.
fn has_nullable_loop(n: &Node) -> bool {
    match n {
        Node::Repeat { node, max, .. } => {
            (max.is_none_or(|m| m > 1) && node.nullable()) || has_nullable_loop(node)
        }
        Node::Concat(v) | Node::Alternate(v) => v.iter().any(has_nullable_loop),
        Node::Group { node, .. } => has_nullable_loop(node),
        _ => false,
    }
}

fn spans_ours(re: &Regex, hay: &str, engine: Engine) -> Option<Option<Spans>> {
    let mut st = Stats::default();
    match re.captures_with(hay, engine, &mut st) {
        Err(_) => None, // предел шагов: этот случай откатом не проверяем
        Ok(c) => Some(c.map(|c| {
            trim(
                (0..c.len())
                    .map(|i| c.get(i).map(|m| (m.start(), m.end())))
                    .collect(),
            )
        })),
    }
}

fn spans_ref(re: &regex::Regex, hay: &str) -> Option<Spans> {
    re.captures(hay).map(|c| {
        trim(
            (0..c.len())
                .map(|i| c.get(i).map(|m| (m.start(), m.end())))
                .collect(),
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig { max_shrink_iters: 2000, ..ProptestConfig::default() })]

    /// Совпадение и все группы — как у крейта regex.
    #[test]
    fn pike_matches_reference_on_captures(pat in regex_strategy(), hay in haystack()) {
        let Ok(reference) = regex::Regex::new(&pat) else { return Ok(()) };
        let ours = Regex::new(&pat).unwrap_or_else(|e| panic!("крейт regex принял /{pat}/, а мы нет: {e}"));
        prop_assert_eq!(spans_ours(&ours, &hay, Engine::Pike).unwrap(), spans_ref(&reference, &hay), "шаблон /{}/ текст {:?}", pat, hay);
    }

    /// Итерация по всем совпадениям (включая пустые) — как у крейта regex.
    #[test]
    fn pike_matches_reference_on_iteration(pat in regex_strategy(), hay in haystack()) {
        let Ok(reference) = regex::Regex::new(&pat) else { return Ok(()) };
        let ours = Regex::new(&pat).unwrap();
        let a: Vec<_> = ours.find_iter(&hay).map(|m| m.range()).collect();
        let b: Vec<_> = reference.find_iter(&hay).map(|m| m.range()).collect();
        prop_assert_eq!(a, b, "шаблон /{}/ текст {:?}", pat, hay);
    }

    /// Pike и откат с запоминанием выдают одно и то же вплоть до групп. Классический откат (семантика Perl) —
    /// то же самое для выражений без циклов над возможно пустым телом; в таких циклах он вправе отличаться.
    #[test]
    fn all_three_engines_agree(pat in regex_strategy(), hay in haystack()) {
        let Ok(re) = Regex::new(&pat) else { return Ok(()) };
        let pike = spans_ours(&re, &hay, Engine::Pike).unwrap();
        let classic = Engine::Backtrack(Limits { max_steps: 200_000, memoize: false });
        let memo = Engine::Backtrack(Limits { max_steps: 200_000, memoize: true });
        if let Some(r) = spans_ours(&re, &hay, classic) {
            if !has_nullable_loop(&parse(&pat).unwrap().node) {
                prop_assert_eq!(&r, &pike, "классический откат ≠ Pike: /{}/ на {:?}", pat, hay);
            }
        }
        if let Some(r) = spans_ours(&re, &hay, memo) {
            prop_assert_eq!(&r, &pike, "откат с запоминанием ≠ Pike: /{}/ на {:?}", pat, hay);
        }
    }

    /// Совпадение — это действительно подстрока по границам символов, а группы лежат внутри него.
    #[test]
    fn spans_are_well_formed(pat in regex_strategy(), hay in haystack()) {
        let Ok(re) = Regex::new(&pat) else { return Ok(()) };
        if let Some(caps) = re.captures(&hay) {
            let whole = caps.get(0).unwrap();
            prop_assert!(whole.start() <= whole.end() && whole.end() <= hay.len());
            for i in 1..caps.len() {
                if let Some(g) = caps.get(i) {
                    prop_assert!(whole.start() <= g.start() && g.start() <= g.end() && g.end() <= whole.end(), "группа {} вне совпадения", i);
                }
            }
        }
    }

    /// Произвольный мусор вместо шаблона: ни паники, ни зависания — только ошибка или результат.
    #[test]
    fn arbitrary_garbage_never_panics(pat in r"[ab()\[\]{}|*+?.^$\\,\-0-9dwsbDWSB:<>=!P]{0,24}", hay in haystack()) {
        if let Ok(re) = Regex::new(&pat) {
            let _ = re.captures(&hay);
            let classic = Engine::Backtrack(Limits { max_steps: 50_000, memoize: false });
            let mut st = Stats::default();
            let _ = re.captures_with(&hay, classic, &mut st);
            let _ = re.find_iter(&hay).count();
        }
    }

    /// Совпадение синтаксиса: что отвергает крейт regex из нашей грамматики, то отвергаем и мы, и наоборот.
    #[test]
    fn syntax_acceptance_agrees_on_generated_patterns(pat in regex_strategy()) {
        prop_assert_eq!(Regex::new(&pat).is_ok(), regex::Regex::new(&pat).is_ok(), "шаблон /{}/", pat);
    }
}
