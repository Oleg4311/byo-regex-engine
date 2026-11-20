//! Виртуальная машина Пайка (Pike VM): моделирует все возможные пути одновременно, по одному символу текста за шаг.
//!
//! Время работы — O(длина текста × размер программы) в худшем случае, без отката и без экспоненциальных взрывов.
//! Порядок потоков в списке — это приоритет (жадность, порядок альтернатив), поэтому результат совпадает
//! с «первым слева, первым по приоритету» совпадением, как у Perl и крейта `regex`.

use crate::ast::Assertion;
use crate::program::{Inst, Program};

/// Значение «ячейка захвата не заполнена».
pub const NONE: usize = usize::MAX;

/// Счётчик работы: сколько раз машина выполнила инструкцию для потока. Нужен для честного сравнения с откатом.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub steps: u64,
}

pub(crate) fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn assertion_holds(a: Assertion, hay: &str, pos: usize) -> bool {
    match a {
        Assertion::Start => pos == 0,
        Assertion::End => pos == hay.len(),
        Assertion::WordBoundary | Assertion::NotWordBoundary => {
            let before = hay[..pos].chars().next_back();
            let after = hay[pos..].chars().next();
            let boundary = is_word(before) != is_word(after);
            boundary == matches!(a, Assertion::WordBoundary)
        }
    }
}

struct Thread {
    pc: usize,
    caps: Box<[usize]>,
}

enum Frame {
    Pc(usize),
    Restore(usize, usize),
}

struct List {
    threads: Vec<Thread>,
}

struct Vm<'a> {
    prog: &'a Program,
    hay: &'a str,
    /// `seen[pc] == поколение` — инструкция уже добавлена в список этого шага (защита от дублей и от зацикливания).
    seen: Vec<u64>,
    generation: u64,
    stack: Vec<Frame>,
    stats: &'a mut Stats,
}

impl Vm<'_> {
    /// Добавляет в список поток и всё, что достижимо из него без чтения символа. Явный стек — без рекурсии.
    fn add(
        &mut self,
        list: &mut List,
        generation: u64,
        pc0: usize,
        pos: usize,
        caps: &mut [usize],
    ) {
        self.stack.push(Frame::Pc(pc0));
        while let Some(frame) = self.stack.pop() {
            let pc = match frame {
                Frame::Restore(slot, old) => {
                    caps[slot] = old;
                    continue;
                }
                Frame::Pc(pc) => pc,
            };
            if self.seen[pc] == generation {
                continue;
            }
            self.seen[pc] = generation;
            self.stats.steps += 1;
            match &self.prog.insts[pc] {
                Inst::Jmp(t) => self.stack.push(Frame::Pc(*t)),
                Inst::Split(a, b) => {
                    self.stack.push(Frame::Pc(*b));
                    self.stack.push(Frame::Pc(*a)); // первым обрабатывается приоритетный путь
                }
                Inst::Save(slot) => {
                    self.stack.push(Frame::Restore(*slot, caps[*slot]));
                    caps[*slot] = pos;
                    self.stack.push(Frame::Pc(pc + 1));
                }
                Inst::EmptyCheck { slot, empty, again } => {
                    self.stack
                        .push(Frame::Pc(if caps[*slot] == pos { *empty } else { *again }));
                }
                Inst::Assert(a) => {
                    if assertion_holds(*a, self.hay, pos) {
                        self.stack.push(Frame::Pc(pc + 1));
                    }
                }
                Inst::Char(_) | Inst::Class(_) | Inst::Match => list.threads.push(Thread {
                    pc,
                    caps: caps.into(),
                }),
            }
        }
    }
}

/// Ищет первое совпадение, начиная с байтовой позиции `start` (она должна быть границей символа).
/// Возвращает ячейки захвата: `[начало0, конец0, начало1, конец1, ...]`, `NONE` — группа не участвовала.
pub fn exec(prog: &Program, hay: &str, start: usize, stats: &mut Stats) -> Option<Vec<usize>> {
    let nslots = prog.slots;
    let mut vm = Vm {
        prog,
        hay,
        seen: vec![0; prog.insts.len()],
        generation: 0,
        stack: Vec::new(),
        stats,
    };
    let mut clist = List {
        threads: Vec::new(),
    };
    let mut nlist = List {
        threads: Vec::new(),
    };
    let mut gen_c = vm.next_generation();
    let mut matched: Option<Vec<usize>> = None;
    let mut pos = start;

    loop {
        // Пока совпадения нет, на каждой позиции стартует новый поток с самым низким приоритетом:
        // так получается поиск «первого слева» без отдельного прохода для каждой стартовой позиции.
        if matched.is_none() {
            let mut caps = vec![NONE; nslots];
            vm.add(&mut clist, gen_c, 0, pos, &mut caps);
        }
        if clist.threads.is_empty() {
            // потоков нет: если совпадение уже найдено — конец; иначе сдвигаемся на символ и пробуем стартовать заново
            if matched.is_some() {
                break;
            }
            match hay[pos..].chars().next() {
                Some(c) => {
                    pos += c.len_utf8();
                    gen_c = vm.next_generation();
                    continue;
                }
                None => break,
            }
        }
        let ch = hay[pos..].chars().next();
        let next_pos = pos + ch.map_or(0, char::len_utf8);
        let gen_n = vm.next_generation();

        for t in std::mem::take(&mut clist.threads) {
            vm.stats.steps += 1;
            match &prog.insts[t.pc] {
                Inst::Char(x) => {
                    if ch == Some(*x) {
                        let mut caps = t.caps;
                        vm.add(&mut nlist, gen_n, t.pc + 1, next_pos, &mut caps);
                    }
                }
                Inst::Class(cl) => {
                    if ch.is_some_and(|c| cl.contains(c)) {
                        let mut caps = t.caps;
                        vm.add(&mut nlist, gen_n, t.pc + 1, next_pos, &mut caps);
                    }
                }
                Inst::Match => {
                    // Всё, что ниже по приоритету, отбрасываем: совпадение уже найдено.
                    matched = Some(t.caps.into_vec());
                    break;
                }
                _ => unreachable!("в списке потоков только инструкции чтения и Match"),
            }
        }
        std::mem::swap(&mut clist, &mut nlist);
        nlist.threads.clear();
        gen_c = gen_n;
        if ch.is_none() {
            break;
        }
        pos = next_pos;
    }
    matched
}

impl Vm<'_> {
    fn next_generation(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}
