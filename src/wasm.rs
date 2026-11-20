//! Интерфейс для WebAssembly без внешних зависимостей: простые C-функции и JSON в линейной памяти.
//!
//! JS выделяет память через `alloc`, записывает туда шаблон и текст, вызывает `analyze`
//! и читает ответ: первые 4 байта — длина (little endian), дальше JSON в UTF-8. Память ответа освобождается через `dealloc`.
//! Те же функции работают и на обычной платформе (так их проверяют юнит-тесты).

use crate::{Engine, Limits, Regex, Stats};
use std::fmt::Write;

/// Выделяет буфер для обмена с JS.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Освобождает буфер, выделенный `alloc` (или вернувшийся из `analyze`: длина = 4 + длина JSON).
///
/// # Safety
/// `ptr` и `len` должны совпадать с выданными `alloc` или `analyze`, и буфер нельзя освобождать дважды.
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Выполняет поиск и возвращает указатель на `[длина:u32 LE][json]`.
/// `engine`: 0 — Pike, 1 — откат, 2 — откат с запоминанием. `max_steps` — предел шагов отката (0 — 10 млн).
///
/// # Safety
/// Указатели должны указывать на валидные участки памяти указанной длины (их выдаёт `alloc`).
#[no_mangle]
pub unsafe extern "C" fn analyze(
    pat: *const u8,
    pat_len: usize,
    hay: *const u8,
    hay_len: usize,
    engine: u32,
    max_steps: u32,
) -> *mut u8 {
    let pat = std::slice::from_raw_parts(pat, pat_len);
    let hay = std::slice::from_raw_parts(hay, hay_len);
    let json = analyze_json(
        &String::from_utf8_lossy(pat),
        &String::from_utf8_lossy(hay),
        engine,
        max_steps,
    );
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(json.as_bytes());
    let mut boxed = out.into_boxed_slice();
    let p = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    p
}

/// Ядро `analyze` на безопасных типах.
pub fn analyze_json(pattern: &str, hay: &str, engine: u32, max_steps: u32) -> String {
    let re = match Regex::new(pattern) {
        Ok(re) => re,
        Err(e) => {
            let mut s = String::from("{\"ok\":false,\"pos\":");
            let _ = write!(s, "{},\"error\":", e.pos);
            json_str(&mut s, &e.to_string());
            s.push('}');
            return s;
        }
    };
    let limits = Limits {
        max_steps: if max_steps == 0 {
            10_000_000
        } else {
            u64::from(max_steps)
        },
        memoize: engine == 2,
    };
    let eng = if engine == 0 {
        Engine::Pike
    } else {
        Engine::Backtrack(limits)
    };
    let mut stats = Stats::default();
    let result = re.all_captures_with(hay, eng, &mut stats);

    let mut s = String::from("{\"ok\":true,\"groups\":");
    let _ = write!(
        s,
        "{},\"steps\":{},\"program\":",
        re.captures_len(),
        stats.steps
    );
    json_str(&mut s, &re.program().to_string());
    s.push_str(",\"names\":[");
    for (i, (n, idx)) in re.program().names.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('[');
        json_str(&mut s, n);
        let _ = write!(s, ",{idx}]");
    }
    s.push_str("],");
    match result {
        Err(_) => s.push_str("\"limit\":true,\"matches\":[]"),
        Ok(all) => {
            s.push_str("\"limit\":false,\"matches\":[");
            for (i, caps) in all.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                s.push('[');
                for g in 0..caps.len() {
                    if g > 0 {
                        s.push(',');
                    }
                    match caps.get(g) {
                        Some(m) => {
                            let _ = write!(s, "[{},{}]", m.start(), m.end());
                        }
                        None => s.push_str("null"),
                    }
                }
                s.push(']');
            }
            s.push(']');
        }
    }
    s.push('}');
    s
}

fn json_str(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
