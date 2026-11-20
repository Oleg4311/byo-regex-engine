//! Дерево разбора регулярного выражения и множества символов.

use std::fmt;

/// Множество символов: отсортированные непересекающиеся диапазоны Unicode-скаляров.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Class {
    ranges: Vec<(char, char)>,
}

const MAX_SCALAR: u32 = 0x10FFFF;

impl Class {
    pub fn new(mut ranges: Vec<(char, char)>) -> Class {
        ranges.sort_unstable();
        let mut merged: Vec<(char, char)> = Vec::with_capacity(ranges.len());
        for (lo, hi) in ranges {
            if let Some(last) = merged.last_mut() {
                // склеиваем пересекающиеся и соседние диапазоны (a-c и d-f → a-f)
                if (lo as u32) <= (last.1 as u32).saturating_add(1) {
                    if hi > last.1 {
                        last.1 = hi;
                    }
                    continue;
                }
            }
            merged.push((lo, hi));
        }
        Class { ranges: merged }
    }

    pub fn single(c: char) -> Class {
        Class {
            ranges: vec![(c, c)],
        }
    }

    pub fn ranges(&self) -> &[(char, char)] {
        &self.ranges
    }

    pub fn contains(&self, c: char) -> bool {
        self.ranges
            .binary_search_by(|&(lo, hi)| {
                if c < lo {
                    std::cmp::Ordering::Greater
                } else if c > hi {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    /// Дополнение до всех Unicode-скаляров (суррогатов в `char` не бывает, их пропускаем).
    pub fn negate(&self) -> Class {
        let mut out = Vec::new();
        let mut next: u32 = 0;
        for &(lo, hi) in &self.ranges {
            if (lo as u32) > next {
                push_scalar_range(&mut out, next, lo as u32 - 1);
            }
            next = hi as u32 + 1;
        }
        if next <= MAX_SCALAR {
            push_scalar_range(&mut out, next, MAX_SCALAR);
        }
        Class { ranges: out }
    }

    pub fn digit() -> Class {
        Class::new(vec![('0', '9')])
    }
    pub fn word() -> Class {
        Class::new(vec![('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')])
    }
    pub fn space() -> Class {
        Class::new(vec![('\t', '\r'), (' ', ' ')])
    }
    /// Точка: любой символ, кроме перевода строки.
    pub fn dot() -> Class {
        Class::single('\n').negate()
    }
}

/// Добавляет диапазон кодовых точек, разрезая его вокруг суррогатов D800–DFFF.
fn push_scalar_range(out: &mut Vec<(char, char)>, lo: u32, hi: u32) {
    const SUR_LO: u32 = 0xD800;
    const SUR_HI: u32 = 0xDFFF;
    let mut push = |a: u32, b: u32| {
        if a <= b {
            if let (Some(x), Some(y)) = (char::from_u32(a), char::from_u32(b)) {
                out.push((x, y));
            }
        }
    };
    if hi < SUR_LO || lo > SUR_HI {
        push(lo, hi);
    } else {
        if lo < SUR_LO {
            push(lo, SUR_LO - 1);
        }
        if hi > SUR_HI {
            push(SUR_HI + 1, hi);
        }
    }
}

/// Утверждения нулевой ширины.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assertion {
    /// `^` — начало текста.
    Start,
    /// `$` — конец текста.
    End,
    /// `\b` — граница слова (слово — ASCII-буквы, цифры и `_`).
    WordBoundary,
    /// `\B` — не граница слова.
    NotWordBoundary,
}

/// Узел дерева разбора.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Empty,
    Char(char),
    Class(Class),
    Assert(Assertion),
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
    /// Группа; `index` — номер захвата (None для `(?:...)`).
    Group {
        node: Box<Node>,
        index: Option<usize>,
    },
}

impl Node {
    /// Может ли узел совпасть с пустой строкой.
    pub fn nullable(&self) -> bool {
        match self {
            Node::Empty | Node::Assert(_) => true,
            Node::Char(_) | Node::Class(_) => false,
            Node::Concat(v) => v.iter().all(Node::nullable),
            Node::Alternate(v) => v.iter().any(Node::nullable),
            Node::Repeat { node, min, .. } => *min == 0 || node.nullable(),
            Node::Group { node, .. } => node.nullable(),
        }
    }
}

/// Результат разбора: дерево, число групп захвата (без нулевой) и имена групп.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub node: Node,
    pub groups: usize,
    pub names: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedEnd,
    UnmatchedOpenParen,
    UnmatchedCloseParen,
    NothingToRepeat,
    InvalidEscape(char),
    InvalidClass(&'static str),
    InvalidRepetition(&'static str),
    Unsupported(&'static str),
    DuplicateGroupName(String),
    TooDeep,
    TooBig,
}

/// Ошибка разбора или компиляции: позиция (в символах) и причина.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub pos: usize,
    pub kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrorKind::*;
        let msg: String = match &self.kind {
            UnexpectedEnd => "неожиданный конец выражения".into(),
            UnmatchedOpenParen => "не закрыта скобка «(»".into(),
            UnmatchedCloseParen => "лишняя закрывающая скобка «)»".into(),
            NothingToRepeat => "перед квантификатором нечего повторять".into(),
            InvalidEscape(c) => format!("неизвестная escape-последовательность «\\{c}»"),
            InvalidClass(why) => format!("ошибка в классе символов: {why}"),
            InvalidRepetition(why) => format!("ошибка в квантификаторе {{n,m}}: {why}"),
            Unsupported(what) => format!("не поддерживается: {what}"),
            DuplicateGroupName(n) => format!("имя группы «{n}» повторяется"),
            TooDeep => "слишком глубокая вложенность скобок".into(),
            TooBig => "выражение после раскрытия повторов слишком велико".into(),
        };
        write!(f, "позиция {}: {}", self.pos, msg)
    }
}

impl std::error::Error for Error {}
