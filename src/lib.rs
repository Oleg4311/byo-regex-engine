//! Движок регулярных выражений с нуля.
//!
//! Конвейер: разбор ([`parse`]) → дерево ([`ast`]) → байткод ([`program`]) → выполнение одним из двух движков:
//! виртуальной машиной Пайка ([`pike`], линейное время) или классическим откатом ([`backtrack`], для сравнения).
//!
//! ```
//! use byo_regex::Regex;
//! let re = Regex::new(r"(?P<year>\d{4})-(?P<month>\d{2})").unwrap();
//! let caps = re.captures("дата: 2026-11-20").unwrap();
//! assert_eq!(caps.name("year").unwrap().as_str(), "2026");
//! assert_eq!(&caps[2], "11");
//! ```

pub mod ast;
pub mod backtrack;
pub mod parse;
pub mod pike;
pub mod program;
pub mod wasm;

use std::ops::Index;

pub use ast::{Error, ErrorKind};
pub use backtrack::{Limits, Outcome};
pub use pike::Stats;
pub use program::{Inst, Program};

/// Какой движок выполняет поиск.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Виртуальная машина Пайка: линейное время, ответ всегда.
    Pike,
    /// Откат с пределом шагов.
    Backtrack(Limits),
}

/// Позиция совпадения (байтовые смещения в тексте).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match<'h> {
    hay: &'h str,
    start: usize,
    end: usize,
}

impl<'h> Match<'h> {
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn end(&self) -> usize {
        self.end
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }
    pub fn as_str(&self) -> &'h str {
        &self.hay[self.start..self.end]
    }
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Результат поиска с группами захвата. Группа 0 — всё совпадение.
#[derive(Debug, Clone)]
pub struct Captures<'h> {
    hay: &'h str,
    groups: usize,
    slots: Vec<usize>,
    names: Vec<(String, usize)>,
}

impl<'h> Captures<'h> {
    pub fn get(&self, i: usize) -> Option<Match<'h>> {
        if i >= self.groups {
            return None;
        }
        let (s, e) = (self.slots[2 * i], self.slots[2 * i + 1]);
        if s == pike::NONE || e == pike::NONE {
            return None;
        }
        Some(Match {
            hay: self.hay,
            start: s,
            end: e,
        })
    }
    pub fn name(&self, name: &str) -> Option<Match<'h>> {
        let (_, i) = self.names.iter().find(|(n, _)| n == name)?;
        self.get(*i)
    }
    /// Число групп, включая нулевую.
    pub fn len(&self) -> usize {
        self.groups
    }
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl Index<usize> for Captures<'_> {
    type Output = str;
    fn index(&self, i: usize) -> &str {
        self.get(i).map_or_else(
            || panic!("группа {i} не участвовала в совпадении"),
            |m| m.as_str(),
        )
    }
}

/// Откат исчерпал предел шагов, не найдя ответа.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepLimitExceeded;

impl std::fmt::Display for StepLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("превышен предел шагов поиска с откатом")
    }
}
impl std::error::Error for StepLimitExceeded {}

/// Скомпилированное регулярное выражение.
#[derive(Debug, Clone)]
pub struct Regex {
    pattern: String,
    prog: Program,
}

impl Regex {
    pub fn new(pattern: &str) -> Result<Regex, Error> {
        let parsed = parse::parse(pattern)?;
        let prog = program::compile(&parsed)?;
        Ok(Regex {
            pattern: pattern.to_string(),
            prog,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Байткод — для изучения и отладки (`Display` печатает дизассемблированный листинг).
    pub fn program(&self) -> &Program {
        &self.prog
    }

    /// Число групп захвата, включая нулевую.
    pub fn captures_len(&self) -> usize {
        self.prog.groups + 1
    }

    pub fn is_match(&self, hay: &str) -> bool {
        self.find(hay).is_some()
    }

    pub fn find<'h>(&self, hay: &'h str) -> Option<Match<'h>> {
        self.captures(hay).and_then(|c| c.get(0))
    }

    pub fn captures<'h>(&self, hay: &'h str) -> Option<Captures<'h>> {
        self.captures_at(hay, 0)
    }

    /// Поиск с байтовой позиции `start` (граница символа); `^` и `\b` видят весь текст.
    pub fn captures_at<'h>(&self, hay: &'h str, start: usize) -> Option<Captures<'h>> {
        let mut stats = Stats::default();
        self.captures_at_with(hay, start, Engine::Pike, &mut stats)
            .ok()
            .flatten()
    }

    /// Поиск выбранным движком со счётчиком шагов. Ошибка — только у отката при превышении предела.
    pub fn captures_with<'h>(
        &self,
        hay: &'h str,
        engine: Engine,
        stats: &mut Stats,
    ) -> Result<Option<Captures<'h>>, StepLimitExceeded> {
        self.captures_at_with(hay, 0, engine, stats)
    }

    /// Поиск с позиции `start` выбранным движком со счётчиком шагов.
    pub fn captures_at_with<'h>(
        &self,
        hay: &'h str,
        start: usize,
        engine: Engine,
        stats: &mut Stats,
    ) -> Result<Option<Captures<'h>>, StepLimitExceeded> {
        if start > hay.len() || !hay.is_char_boundary(start) {
            return Ok(None);
        }
        let slots = match engine {
            Engine::Pike => pike::exec(&self.prog, hay, start, stats),
            Engine::Backtrack(limits) => {
                match backtrack::exec(&self.prog, hay, start, limits, stats) {
                    Outcome::Match(s) => Some(s),
                    Outcome::NoMatch => None,
                    Outcome::StepLimit => return Err(StepLimitExceeded),
                }
            }
        };
        Ok(slots.map(|slots| Captures {
            hay,
            groups: self.prog.groups + 1,
            slots,
            names: self.prog.names.clone(),
        }))
    }

    /// Все непересекающиеся совпадения слева направо. Пустое совпадение сразу после предыдущего не выдаётся.
    pub fn find_iter<'r, 'h>(&'r self, hay: &'h str) -> impl Iterator<Item = Match<'h>> + 'r
    where
        'h: 'r,
    {
        self.captures_iter(hay).filter_map(|c| c.get(0))
    }

    pub fn captures_iter<'r, 'h>(&'r self, hay: &'h str) -> CaptureMatches<'r, 'h> {
        CaptureMatches {
            re: self,
            hay,
            state: IterState::default(),
        }
    }

    /// Все совпадения выбранным движком (для откатного движка — с общим счётчиком шагов).
    pub fn all_captures_with<'h>(
        &self,
        hay: &'h str,
        engine: Engine,
        stats: &mut Stats,
    ) -> Result<Vec<Captures<'h>>, StepLimitExceeded> {
        let mut st = IterState::default();
        let mut out = Vec::new();
        while let Some(c) = self.advance(hay, &mut st, engine, stats)? {
            out.push(c);
        }
        Ok(out)
    }

    fn advance<'h>(
        &self,
        hay: &'h str,
        st: &mut IterState,
        engine: Engine,
        stats: &mut Stats,
    ) -> Result<Option<Captures<'h>>, StepLimitExceeded> {
        while !st.done {
            let Some(caps) = self.captures_at_with(hay, st.pos, engine, stats)? else {
                st.done = true;
                return Ok(None);
            };
            let Some(m) = caps.get(0) else {
                st.done = true;
                return Ok(None);
            };
            if m.is_empty() {
                // пустое совпадение: следующий поиск начнём на символ правее, иначе зациклимся
                match hay[m.end()..].chars().next() {
                    Some(c) => st.pos = m.end() + c.len_utf8(),
                    None => st.done = true,
                }
                if st.last_end == Some(m.end()) {
                    continue; // пустое совпадение вплотную к предыдущему не считается
                }
            } else {
                st.pos = m.end();
            }
            st.last_end = Some(m.end());
            return Ok(Some(caps));
        }
        Ok(None)
    }
}

#[derive(Default)]
struct IterState {
    pos: usize,
    last_end: Option<usize>,
    done: bool,
}

/// Итератор по совпадениям с группами (движок Пайка).
pub struct CaptureMatches<'r, 'h> {
    re: &'r Regex,
    hay: &'h str,
    state: IterState,
}

impl<'h> Iterator for CaptureMatches<'_, 'h> {
    type Item = Captures<'h>;

    fn next(&mut self) -> Option<Captures<'h>> {
        let mut stats = Stats::default();
        self.re
            .advance(self.hay, &mut self.state, Engine::Pike, &mut stats)
            .ok()
            .flatten()
    }
}
