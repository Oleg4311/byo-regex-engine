//! Разбор синтаксиса регулярных выражений рекурсивным спуском.
//!
//! Поддерживается: литералы, `.`, классы `[...]`, `\d \w \s` и их отрицания, `\b \B`, `^ $`,
//! группы `(...)`, `(?:...)`, `(?P<имя>...)`, `(?<имя>...)`, альтернатива `|`,
//! квантификаторы `* + ? {n} {n,} {n,m}` и их «ленивые» версии с суффиксом `?`.
//! Не поддерживается: обратные ссылки, просмотр вперёд и назад, флаги `(?i)`, именованные
//! классы `[[:alpha:]]` — всё, что нельзя выполнить за линейное время (или не нужно для учебного движка).

use crate::ast::{Assertion, Class, Error, ErrorKind, Node, Parsed};

/// Максимальная вложенность скобок: защита от переполнения стека на вредном вводе.
pub const MAX_DEPTH: usize = 250;
/// Максимальное значение в `{n,m}`.
pub const MAX_REPEAT: u32 = 1000;

pub fn parse(pattern: &str) -> Result<Parsed, Error> {
    let mut p = Parser {
        chars: pattern.chars().collect(),
        pos: 0,
        groups: 0,
        names: Vec::new(),
    };
    let node = p.parse_alternate(0)?;
    if p.pos < p.chars.len() {
        // единственное, на чём parse_alternate останавливается на верхнем уровне, — лишняя «)»
        return Err(Error {
            pos: p.pos,
            kind: ErrorKind::UnmatchedCloseParen,
        });
    }
    Ok(Parsed {
        node,
        groups: p.groups,
        names: p.names,
    })
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    groups: usize,
    names: Vec<(String, usize)>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn err<T>(&self, kind: ErrorKind) -> Result<T, Error> {
        Err(Error {
            pos: self.pos,
            kind,
        })
    }

    fn parse_alternate(&mut self, depth: usize) -> Result<Node, Error> {
        if depth > MAX_DEPTH {
            return self.err(ErrorKind::TooDeep);
        }
        let mut branches = vec![self.parse_concat(depth)?];
        while self.eat('|') {
            branches.push(self.parse_concat(depth)?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alternate(branches)
        })
    }

    fn parse_concat(&mut self, depth: usize) -> Result<Node, Error> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            items.push(self.parse_repeat(depth)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    /// Атом и цепочка квантификаторов: `a**` и `a{2}{3}` допустимы и вкладываются друг в друга (как в крейте regex).
    fn parse_repeat(&mut self, depth: usize) -> Result<Node, Error> {
        let mut node = self.parse_atom(depth)?;
        while let Some((min, max)) = self.parse_quantifier()? {
            let greedy = !self.eat('?');
            node = Node::Repeat {
                node: Box::new(node),
                min,
                max,
                greedy,
            };
        }
        Ok(node)
    }

    fn parse_quantifier(&mut self) -> Result<Option<(u32, Option<u32>)>, Error> {
        match self.peek() {
            Some('*') => {
                self.pos += 1;
                Ok(Some((0, None)))
            }
            Some('+') => {
                self.pos += 1;
                Ok(Some((1, None)))
            }
            Some('?') => {
                self.pos += 1;
                Ok(Some((0, Some(1))))
            }
            Some('{') => {
                self.pos += 1;
                let min = self.parse_number()?;
                let max = if self.eat(',') {
                    if self.peek() == Some('}') {
                        None
                    } else {
                        Some(self.parse_number()?)
                    }
                } else {
                    Some(min)
                };
                if !self.eat('}') {
                    return self.err(ErrorKind::InvalidRepetition("ожидалась «}»"));
                }
                if let Some(m) = max {
                    if m < min {
                        return self.err(ErrorKind::InvalidRepetition("минимум больше максимума"));
                    }
                }
                if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT) {
                    return self.err(ErrorKind::InvalidRepetition("число повторов больше 1000"));
                }
                Ok(Some((min, max)))
            }
            _ => Ok(None),
        }
    }

    fn parse_number(&mut self) -> Result<u32, Error> {
        let start = self.pos;
        let mut n: u32 = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n.saturating_mul(10).saturating_add(d);
            self.pos += 1;
        }
        if self.pos == start {
            return self.err(ErrorKind::InvalidRepetition("ожидалось число"));
        }
        Ok(n)
    }

    fn parse_atom(&mut self, depth: usize) -> Result<Node, Error> {
        let c = match self.peek() {
            Some(c) => c,
            None => return self.err(ErrorKind::UnexpectedEnd),
        };
        match c {
            '(' => self.parse_group(depth),
            '[' => self.parse_class().map(Node::Class),
            '.' => {
                self.pos += 1;
                Ok(Node::Class(Class::dot()))
            }
            '^' => {
                self.pos += 1;
                Ok(Node::Assert(Assertion::Start))
            }
            '$' => {
                self.pos += 1;
                Ok(Node::Assert(Assertion::End))
            }
            '\\' => self.parse_escape(),
            '*' | '+' | '?' | '{' => self.err(ErrorKind::NothingToRepeat),
            _ => {
                self.pos += 1;
                Ok(Node::Char(c))
            }
        }
    }

    fn parse_group(&mut self, depth: usize) -> Result<Node, Error> {
        let open = self.pos;
        self.pos += 1; // «(»
        let mut index = None;
        if self.eat('?') {
            match self.peek() {
                Some(':') => self.pos += 1,
                Some('P') if self.peek_at(1) == Some('<') => {
                    self.pos += 2;
                    index = Some(self.parse_group_name()?);
                }
                Some('<') if !matches!(self.peek_at(1), Some('=') | Some('!')) => {
                    self.pos += 1;
                    index = Some(self.parse_group_name()?);
                }
                Some('=') | Some('!') | Some('<') => {
                    return self.err(ErrorKind::Unsupported("просмотр вперёд и назад"))
                }
                _ => return self.err(ErrorKind::Unsupported("флаги и расширения (?...)")),
            }
        } else {
            self.groups += 1;
            index = Some(self.groups);
        }
        let inner = self.parse_alternate(depth + 1)?;
        if !self.eat(')') {
            return Err(Error {
                pos: open,
                kind: ErrorKind::UnmatchedOpenParen,
            });
        }
        Ok(Node::Group {
            node: Box::new(inner),
            index,
        })
    }

    /// Читает имя до «>» и регистрирует именованную группу; возвращает её номер.
    fn parse_group_name(&mut self) -> Result<usize, Error> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == '>' {
                break;
            }
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return self.err(ErrorKind::InvalidClass(
                    "недопустимый символ в имени группы",
                ));
            }
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
            return self.err(ErrorKind::InvalidClass(
                "пустое имя группы или имя начинается с цифры",
            ));
        }
        if !self.eat('>') {
            return self.err(ErrorKind::UnexpectedEnd);
        }
        if self.names.iter().any(|(n, _)| *n == name) {
            return self.err(ErrorKind::DuplicateGroupName(name));
        }
        self.groups += 1;
        self.names.push((name, self.groups));
        Ok(self.groups)
    }

    fn parse_escape(&mut self) -> Result<Node, Error> {
        self.pos += 1; // «\»
        let c = match self.bump() {
            Some(c) => c,
            None => return self.err(ErrorKind::UnexpectedEnd),
        };
        Ok(match c {
            'd' => Node::Class(Class::digit()),
            'D' => Node::Class(Class::digit().negate()),
            'w' => Node::Class(Class::word()),
            'W' => Node::Class(Class::word().negate()),
            's' => Node::Class(Class::space()),
            'S' => Node::Class(Class::space().negate()),
            'b' => Node::Assert(Assertion::WordBoundary),
            'B' => Node::Assert(Assertion::NotWordBoundary),
            _ => Node::Char(self.literal_escape(c)?),
        })
    }

    /// Escape, обозначающий один символ (общий для обычного режима и классов).
    fn literal_escape(&mut self, c: char) -> Result<char, Error> {
        Ok(match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'f' => '\u{0C}',
            'v' => '\u{0B}',
            'x' => {
                let hi = self.bump().and_then(|c| c.to_digit(16));
                let lo = self.bump().and_then(|c| c.to_digit(16));
                match (hi, lo) {
                    (Some(h), Some(l)) => char::from_u32(h * 16 + l).unwrap(),
                    _ => return self.err(ErrorKind::InvalidEscape('x')),
                }
            }
            c if c.is_ascii_punctuation() || c == ' ' => c,
            c => {
                self.pos -= 1;
                return self.err(ErrorKind::InvalidEscape(c));
            }
        })
    }

    fn parse_class(&mut self) -> Result<Class, Error> {
        let open = self.pos;
        self.pos += 1; // «[»
        let negated = self.eat('^');
        let mut ranges: Vec<(char, char)> = Vec::new();
        let mut first = true;
        loop {
            let c = match self.peek() {
                Some(c) => c,
                None => {
                    return Err(Error {
                        pos: open,
                        kind: ErrorKind::InvalidClass("класс не закрыт «]»"),
                    })
                }
            };
            if c == ']' && !first {
                self.pos += 1;
                break;
            }
            first = false;
            if c == '[' {
                return self.err(ErrorKind::Unsupported("вложенные классы и [[:alpha:]]"));
            }
            match self.class_item()? {
                ClassItem::Set(set) => {
                    // «[\d-z]»: класс не может быть началом диапазона (а «[\d-]» — можно: дефис в конце буквальный)
                    if self.peek() == Some('-') && self.peek_at(1).is_some_and(|n| n != ']') {
                        return self.err(ErrorKind::InvalidClass(
                            "класс \\d, \\w, \\s не может быть границей диапазона",
                        ));
                    }
                    ranges.extend_from_slice(set.ranges());
                }
                ClassItem::Char(lo) => {
                    // «a-z»: дефис, за которым не «]», образует диапазон
                    if self.peek() == Some('-') && self.peek_at(1).is_some_and(|n| n != ']') {
                        self.pos += 1;
                        let hi = match self.class_item()? {
                            ClassItem::Char(h) => h,
                            ClassItem::Set(_) => {
                                return self.err(ErrorKind::InvalidClass(
                                    "класс \\d, \\w, \\s не может быть границей диапазона",
                                ))
                            }
                        };
                        if hi < lo {
                            return self
                                .err(ErrorKind::InvalidClass("начало диапазона больше конца"));
                        }
                        ranges.push((lo, hi));
                    } else {
                        ranges.push((lo, lo));
                    }
                }
            }
        }
        let class = Class::new(ranges);
        Ok(if negated { class.negate() } else { class })
    }

    fn class_item(&mut self) -> Result<ClassItem, Error> {
        let c = match self.bump() {
            Some(c) => c,
            None => return self.err(ErrorKind::InvalidClass("класс не закрыт «]»")),
        };
        if c != '\\' {
            return Ok(ClassItem::Char(c));
        }
        let e = match self.bump() {
            Some(e) => e,
            None => return self.err(ErrorKind::UnexpectedEnd),
        };
        Ok(match e {
            'd' => ClassItem::Set(Class::digit()),
            'D' => ClassItem::Set(Class::digit().negate()),
            'w' => ClassItem::Set(Class::word()),
            'W' => ClassItem::Set(Class::word().negate()),
            's' => ClassItem::Set(Class::space()),
            'S' => ClassItem::Set(Class::space().negate()),
            _ => ClassItem::Char(self.literal_escape(e)?),
        })
    }
}

enum ClassItem {
    Char(char),
    Set(Class),
}
