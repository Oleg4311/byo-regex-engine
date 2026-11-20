//! Сравнение скорости с крейтом `regex` на нескольких шаблонах.
//! Запуск: cargo run --release --example bench
//!
//! Цель — честная картина: движок учебный (Pike VM без DFA, без оптимизаций поиска по литералам),
//! поэтому он заметно медленнее промышленного `regex`, но время растёт линейно с длиной текста.

use std::time::Instant;

fn make_log(lines: usize) -> String {
    let mut s = String::new();
    for i in 0..lines {
        s.push_str(&format!(
            "2026-11-{:02} 12:{:02}:{:02} user{}@example.com GET /api/v1/items/{} 200 {}ms\n",
            1 + i % 28,
            i % 60,
            (i * 7) % 60,
            i % 97,
            i,
            i % 900
        ));
    }
    s
}

fn time<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let mut best = f64::MAX;
    let mut n = 0;
    for _ in 0..3 {
        let t = Instant::now();
        n = f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    (best, n)
}

fn main() {
    let text = make_log(8_000);
    let mb = text.len() as f64 / 1e6;
    let cases: &[(&str, &str)] = &[
        ("дата", r"\d{4}-\d{2}-\d{2}"),
        ("почта", r"[a-z0-9]+@[a-z]+\.com"),
        ("путь", r"/api/v\d+/\w+/\d+"),
        ("альтернатива", r"GET|POST|PUT|DELETE"),
        ("нет совпадений", r"(foo|bar)+baz"),
        ("(a|aa)+b на aaaa…", r"(a|aa)+b"),
    ];
    println!(
        "Текст: {:.2} МБ ({} строк журнала)\n",
        mb,
        text.lines().count()
    );
    println!(
        "{:<22} {:>14} {:>14} {:>10}",
        "шаблон", "byo-regex МБ/с", "regex МБ/с", "во сколько раз"
    );
    for (name, pat) in cases {
        let hay = if *name == "(a|aa)+b на aaaa…" {
            "a".repeat(200_000)
        } else {
            text.clone()
        };
        let mb = hay.len() as f64 / 1e6;
        let ours = byo_regex::Regex::new(pat).unwrap();
        let theirs = regex::Regex::new(pat).unwrap();
        let (t1, n1) = time(|| ours.find_iter(&hay).count());
        let (t2, n2) = time(|| theirs.find_iter(&hay).count());
        assert_eq!(n1, n2, "число совпадений должно совпасть: {pat}");
        println!(
            "{:<22} {:>14.1} {:>14.1} {:>9.0}×",
            name,
            mb / t1,
            mb / t2,
            t1 / t2
        );
    }
}
