//! `bre` — командная строка движка: поиск по строкам (как grep), листинг байткода, сравнение движков.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use byo_regex::{Engine, Limits, Regex, Stats};

const USAGE: &str = "bre — движок регулярных выражений с нуля

Использование:
  bre [флаги] <шаблон> [файл...]      построчный поиск (читает stdin, если файлов нет)
  bre --explain <шаблон>              показать байткод
  bre --version

Флаги:
  -o               печатать только совпавшую часть
  -n               номера строк
  -c               только число подходящих строк
  -v               строки, которые НЕ подходят
  --engine pike|backtrack|memo   движок (по умолчанию pike);
                                 backtrack — классический откат с пределом шагов, memo — откат с запоминанием
  --max-steps N    предел шагов отката (по умолчанию 10000000)
  --stats          после работы вывести число шагов (в stderr)

Код выхода: 0 — есть подходящие строки, 1 — нет, 2 — ошибка (в том числе превышен предел шагов отката).
";

struct Opts {
    only: bool,
    numbers: bool,
    count: bool,
    invert: bool,
    explain: bool,
    stats: bool,
    engine: Engine,
    pattern: String,
    files: Vec<String>,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        only: false,
        numbers: false,
        count: false,
        invert: false,
        explain: false,
        stats: false,
        engine: Engine::Pike,
        pattern: String::new(),
        files: Vec::new(),
    };
    let mut max_steps = 10_000_000u64;
    let mut kind = "pike".to_string();
    // «-on» → «-o -n»: склеенные короткие флаги
    let raw: Vec<String> = std::env::args()
        .skip(1)
        .flat_map(|a| {
            let short = a.len() > 2
                && a.starts_with('-')
                && !a.starts_with("--")
                && a[1..].chars().all(|c| "oncv".contains(c));
            if short {
                a[1..].chars().map(|c| format!("-{c}")).collect::<Vec<_>>()
            } else {
                vec![a]
            }
        })
        .collect();
    let mut args = raw.into_iter();
    let mut positional = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--version" => {
                println!("bre {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "-o" => o.only = true,
            "-n" => o.numbers = true,
            "-c" => o.count = true,
            "-v" => o.invert = true,
            "--explain" => o.explain = true,
            "--stats" => o.stats = true,
            "--engine" => kind = args.next().ok_or("--engine требует значение")?,
            "--max-steps" => {
                max_steps = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--max-steps требует число")?
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("неизвестный флаг {s}")),
            _ => positional.push(a),
        }
    }
    o.engine = match kind.as_str() {
        "pike" => Engine::Pike,
        "backtrack" => Engine::Backtrack(Limits {
            max_steps,
            memoize: false,
        }),
        "memo" => Engine::Backtrack(Limits {
            max_steps,
            memoize: true,
        }),
        other => return Err(format!("неизвестный движок «{other}»")),
    };
    let mut it = positional.into_iter();
    o.pattern = it.next().ok_or("нужен шаблон")?;
    o.files = it.collect();
    Ok(o)
}

fn main() -> ExitCode {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("ошибка: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let re = match Regex::new(&o.pattern) {
        Ok(re) => re,
        Err(e) => {
            eprintln!("ошибка в шаблоне: {e}");
            return ExitCode::from(2);
        }
    };
    if o.explain {
        print!("{}", re.program());
        return ExitCode::SUCCESS;
    }

    let mut inputs: Vec<(String, Box<dyn BufRead>)> = Vec::new();
    if o.files.is_empty() {
        inputs.push(("-".into(), Box::new(io::stdin().lock())));
    } else {
        for f in &o.files {
            match std::fs::File::open(f) {
                Ok(h) => inputs.push((f.clone(), Box::new(io::BufReader::new(h)))),
                Err(e) => {
                    eprintln!("ошибка: {f}: {e}");
                    return ExitCode::from(2);
                }
            }
        }
    }

    let multi = inputs.len() > 1;
    let mut stats = Stats::default();
    let mut matched_lines = 0usize;
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for (name, reader) in inputs {
        for (i, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("ошибка чтения {name}: {e}");
                    return ExitCode::from(2);
                }
            };
            let found = match re.all_captures_with(&line, o.engine, &mut stats) {
                Ok(v) => v,
                Err(e) => {
                    let _ = out.flush();
                    eprintln!("{name}:{}: {e}. Попробуйте --engine pike или memo", i + 1);
                    return ExitCode::from(2);
                }
            };
            let hit = !found.is_empty();
            if hit == o.invert {
                continue;
            }
            matched_lines += 1;
            if o.count {
                continue;
            }
            let prefix = format!(
                "{}{}",
                if multi {
                    format!("{name}:")
                } else {
                    String::new()
                },
                if o.numbers {
                    format!("{}:", i + 1)
                } else {
                    String::new()
                }
            );
            if o.only && !o.invert {
                for c in &found {
                    if let Some(m) = c.get(0) {
                        let _ = writeln!(out, "{prefix}{}", m.as_str());
                    }
                }
            } else {
                let _ = writeln!(out, "{prefix}{line}");
            }
        }
    }
    if o.count {
        let _ = writeln!(out, "{matched_lines}");
    }
    let _ = out.flush();
    if o.stats {
        eprintln!("шагов: {}", stats.steps);
    }
    if matched_lines > 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
