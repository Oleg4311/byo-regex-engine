//! Классический поиск с откатом (как в Perl, Python, Java): идём по первому пути до конца, при неудаче возвращаемся.
//!
//! Нужен как эталон поведения и как наглядный контраст: на выражениях вроде `(a+)+b` время растёт
//! экспоненциально (катастрофический откат), поэтому у поиска есть предел шагов. Режим `memoize`
//! запоминает пары (инструкция, позиция) и тем самым делает откат полиномиальным (так устроен BitState в RE2);
//! результат при этом тот же, что у Pike VM.
//!
//! Оговорка: классический откат без запоминания — это семантика Perl. На выражениях без циклов над возможно
//! пустым телом он совпадает с Pike VM (это проверяется тестами), а в таких циклах (`(.?)*`, `(a|b*)*`) может
//! вернуть другие границы — и групп, и всего совпадения.

use std::collections::HashSet;

use crate::pike::{assertion_holds, Stats, NONE};
use crate::program::{Inst, Program};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Предел числа шагов; при превышении поиск прекращается с `Outcome::StepLimit`.
    pub max_steps: u64,
    /// Запоминать посещённые (инструкция, позиция): убирает экспоненциальный взрыв.
    pub memoize: bool,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_steps: 10_000_000,
            memoize: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Match(Vec<usize>),
    NoMatch,
    /// Исчерпан предел шагов: ответа нет, это не «нет совпадения».
    StepLimit,
}

enum Frame {
    Try(usize, usize),
    Restore(usize, usize),
    LeavePath(usize, usize),
}

/// Ищет первое совпадение, начиная с байтовой позиции `start`.
pub fn exec(prog: &Program, hay: &str, start: usize, limits: Limits, stats: &mut Stats) -> Outcome {
    let nslots = prog.slots;
    let width = hay.len() + 1;
    // Память под таблицу посещённых: инструкции × позиции бит. Ограничиваем, чтобы вредный ввод не съел память.
    let memo_bits = prog.insts.len().saturating_mul(width);
    let mut visited = if limits.memoize {
        if memo_bits > 1 << 31 {
            return Outcome::StepLimit;
        }
        Some(vec![0u64; memo_bits.div_ceil(64)])
    } else {
        None
    };

    let mut pos0 = start;
    loop {
        let mut caps = vec![NONE; nslots];
        match run(prog, hay, pos0, &mut caps, limits, &mut visited, stats) {
            Some(true) => return Outcome::Match(caps),
            Some(false) => {}
            None => return Outcome::StepLimit,
        }
        match hay[pos0..].chars().next() {
            Some(c) => pos0 += c.len_utf8(),
            None => return Outcome::NoMatch,
        }
    }
}

/// Одна попытка с фиксированной начальной позицией. `Some(true)` — совпало, `None` — предел шагов.
fn run(
    prog: &Program,
    hay: &str,
    pos0: usize,
    caps: &mut [usize],
    limits: Limits,
    visited: &mut Option<Vec<u64>>,
    stats: &mut Stats,
) -> Option<bool> {
    let width = hay.len() + 1;
    let mut stack = vec![Frame::Try(0, pos0)];
    // Без запоминания зацикливание на пустых телах `(a*)*` предотвращает множество пар на текущем пути.
    let mut on_path: HashSet<(usize, usize)> = HashSet::new();

    while let Some(frame) = stack.pop() {
        let (mut pc, mut pos) = match frame {
            Frame::Restore(slot, old) => {
                caps[slot] = old;
                continue;
            }
            Frame::LeavePath(pc, pos) => {
                on_path.remove(&(pc, pos));
                continue;
            }
            Frame::Try(pc, pos) => (pc, pos),
        };
        loop {
            stats.steps += 1;
            if stats.steps > limits.max_steps {
                return None;
            }
            if let Some(bits) = visited.as_mut() {
                // Запоминание на КАЖДОЙ инструкции: кто первым пришёл в (инструкция, позиция), тот приоритетнее, и
                // продолжение у всех одинаковое. Это ровно правило Pike VM, поэтому ответы совпадают вплоть до групп.
                let idx = pc * width + pos;
                let (w, m) = (idx / 64, 1u64 << (idx % 64));
                if bits[w] & m != 0 {
                    break;
                }
                bits[w] |= m;
            }
            match &prog.insts[pc] {
                Inst::Char(x) => match hay[pos..].chars().next() {
                    Some(c) if c == *x => {
                        pos += c.len_utf8();
                        pc += 1;
                    }
                    _ => break,
                },
                Inst::Class(cl) => match hay[pos..].chars().next() {
                    Some(c) if cl.contains(c) => {
                        pos += c.len_utf8();
                        pc += 1;
                    }
                    _ => break,
                },
                Inst::Jmp(t) => pc = *t,
                Inst::Split(a, b) => {
                    if visited.is_none() {
                        // без запоминания: защита только от бесконечного цикла по пустым телам
                        if on_path.contains(&(pc, pos)) {
                            break;
                        }
                        on_path.insert((pc, pos));
                        stack.push(Frame::LeavePath(pc, pos));
                    }
                    stack.push(Frame::Try(*b, pos));
                    pc = *a;
                }
                Inst::Save(slot) => {
                    stack.push(Frame::Restore(*slot, caps[*slot]));
                    caps[*slot] = pos;
                    pc += 1;
                }
                Inst::EmptyCheck { slot, empty, again } => {
                    pc = if caps[*slot] == pos { *empty } else { *again }
                }
                Inst::Assert(a) => {
                    if assertion_holds(*a, hay, pos) {
                        pc += 1;
                    } else {
                        break;
                    }
                }
                Inst::Match => return Some(true),
            }
        }
    }
    Some(false)
}
