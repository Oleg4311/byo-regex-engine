//! Компиляция дерева разбора в байткод (программу для виртуальной машины).

use std::fmt;

use crate::ast::{Assertion, Class, Error, ErrorKind, Node, Parsed};

/// Предел размера программы после раскрытия `{n,m}`: `(a{1000}){1000}` не должно съесть всю память.
pub const MAX_PROGRAM: usize = 200_000;

/// Инструкция виртуальной машины.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inst {
    /// Прочитать ровно этот символ.
    Char(char),
    /// Прочитать один символ из множества.
    Class(Class),
    /// Ветвление: сначала пробуем первый адрес, потом второй (порядок задаёт жадность).
    Split(usize, usize),
    Jmp(usize),
    /// Записать текущую позицию в ячейку захвата (2·n — начало группы n, 2·n+1 — конец).
    Save(usize),
    /// Конец итерации цикла с возможно пустым телом: если позиция не сдвинулась с начала итерации
    /// (ячейка `slot`), цикл завершается (`empty`), иначе идёт следующая итерация (`again`).
    EmptyCheck {
        slot: usize,
        empty: usize,
        again: usize,
    },
    Assert(Assertion),
    Match,
}

/// Скомпилированная программа.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub insts: Vec<Inst>,
    /// Число групп захвата без нулевой (она — весь матч).
    pub groups: usize,
    pub names: Vec<(String, usize)>,
    /// Всего ячеек: две на каждую группу (с нулевой) и по одной на каждый цикл с возможно пустым телом.
    pub slots: usize,
}

pub fn compile(parsed: &Parsed) -> Result<Program, Error> {
    let mut c = Compiler {
        insts: Vec::new(),
        slots: 2 * (parsed.groups + 1),
    };
    c.push(Inst::Save(0))?;
    c.node(&parsed.node)?;
    c.push(Inst::Save(1))?;
    c.push(Inst::Match)?;
    Ok(Program {
        insts: c.insts,
        groups: parsed.groups,
        names: parsed.names.clone(),
        slots: c.slots,
    })
}

struct Compiler {
    insts: Vec<Inst>,
    slots: usize,
}

impl Compiler {
    fn push(&mut self, inst: Inst) -> Result<usize, Error> {
        if self.insts.len() >= MAX_PROGRAM {
            return Err(Error {
                pos: 0,
                kind: ErrorKind::TooBig,
            });
        }
        self.insts.push(inst);
        Ok(self.insts.len() - 1)
    }

    fn here(&self) -> usize {
        self.insts.len()
    }

    fn node(&mut self, n: &Node) -> Result<(), Error> {
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.push(Inst::Char(*c))?;
            }
            Node::Class(cl) => {
                self.push(Inst::Class(cl.clone()))?;
            }
            Node::Assert(a) => {
                self.push(Inst::Assert(*a))?;
            }
            Node::Concat(items) => {
                for it in items {
                    self.node(it)?;
                }
            }
            Node::Alternate(branches) => {
                // Split(L1, L2); L1: a; Jmp end; L2: Split(...) ... — цепочка, порядок ветвей = приоритет
                let mut jumps = Vec::new();
                for (i, b) in branches.iter().enumerate() {
                    if i + 1 < branches.len() {
                        let split = self.push(Inst::Split(0, 0))?;
                        self.node(b)?;
                        jumps.push(self.push(Inst::Jmp(0))?);
                        let next = self.here();
                        self.insts[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(b)?;
                    }
                }
                let end = self.here();
                for j in jumps {
                    self.insts[j] = Inst::Jmp(end);
                }
            }
            Node::Group { node, index } => match index {
                Some(i) => {
                    self.push(Inst::Save(2 * i))?;
                    self.node(node)?;
                    self.push(Inst::Save(2 * i + 1))?;
                }
                None => self.node(node)?,
            },
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => self.repeat(node, *min, *max, *greedy)?,
        }
        Ok(())
    }

    fn split(&mut self, body: usize, exit: usize, greedy: bool) -> Inst {
        if greedy {
            Inst::Split(body, exit)
        } else {
            Inst::Split(exit, body)
        }
    }

    fn repeat(
        &mut self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<(), Error> {
        match max {
            None if min == 0 => self.star(node, greedy),
            None => {
                for _ in 0..min - 1 {
                    self.node(node)?;
                }
                self.plus(node, greedy)
            }
            Some(max) => {
                for _ in 0..min {
                    self.node(node)?;
                }
                // x{n,m} = n копий и (m-n) вложенных необязательных: x(x(x)?)? — все выходы ведут в конец
                let mut splits = Vec::new();
                for _ in min..max {
                    splits.push(self.push(Inst::Split(0, 0))?);
                    self.node(node)?;
                }
                let end = self.here();
                for s in splits {
                    let inst = self.split(s + 1, end, greedy);
                    self.insts[s] = inst;
                }
                Ok(())
            }
        }
    }

    /// e*: L1: Split(L2, L3); L2: e; Jmp L1; L3:
    /// Если e может совпасть с пустой строкой, итерация без продвижения завершает цикл (как в Perl и RE2),
    /// иначе цикл зациклился бы: L1: Split(L2, L3); L2: Save(r); e; EmptyCheck(r → L3, иначе L1); L3:
    fn star(&mut self, node: &Node, greedy: bool) -> Result<(), Error> {
        let l1 = self.push(Inst::Split(0, 0))?;
        if node.nullable() {
            let slot = self.slots;
            self.slots += 1;
            self.push(Inst::Save(slot))?;
            self.node(node)?;
            let check = self.push(Inst::EmptyCheck {
                slot,
                empty: 0,
                again: l1,
            })?;
            let l3 = self.here();
            self.insts[check] = Inst::EmptyCheck {
                slot,
                empty: l3,
                again: l1,
            };
            self.insts[l1] = self.split(l1 + 1, l3, greedy);
        } else {
            self.node(node)?;
            self.push(Inst::Jmp(l1))?;
            let l3 = self.here();
            self.insts[l1] = self.split(l1 + 1, l3, greedy);
        }
        Ok(())
    }

    /// e+: L1: e; Split(L1, L2); L2:
    fn plus(&mut self, node: &Node, greedy: bool) -> Result<(), Error> {
        let l1 = self.here();
        self.node(node)?;
        let s = self.push(Inst::Split(0, 0))?;
        let l2 = self.here();
        self.insts[s] = self.split(l1, l2, greedy);
        Ok(())
    }
}

impl fmt::Display for Program {
    /// Дизассемблер: по одной инструкции на строку.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, inst) in self.insts.iter().enumerate() {
            write!(f, "{i:>4}: ")?;
            match inst {
                Inst::Char(c) => writeln!(f, "char {}", show_char(*c))?,
                Inst::Class(cl) => writeln!(f, "class {}", show_class(cl))?,
                Inst::Split(a, b) => writeln!(f, "split {a}, {b}")?,
                Inst::Jmp(t) => writeln!(f, "jmp {t}")?,
                Inst::Save(s) => writeln!(f, "save {s}")?,
                Inst::EmptyCheck { slot, empty, again } => {
                    writeln!(f, "emptycheck {slot}: пусто → {empty}, иначе → {again}")?
                }
                Inst::Assert(a) => writeln!(f, "assert {a:?}")?,
                Inst::Match => writeln!(f, "match")?,
            }
        }
        Ok(())
    }
}

fn show_char(c: char) -> String {
    if c.is_control() || c == ' ' {
        format!("{:?}", c)
    } else {
        format!("'{c}'")
    }
}

fn show_class(cl: &Class) -> String {
    let mut s = String::from("[");
    for &(lo, hi) in cl.ranges() {
        if lo == hi {
            s += &esc(lo);
        } else {
            s += &format!("{}-{}", esc(lo), esc(hi));
        }
        if s.len() > 60 {
            s += "…";
            break;
        }
    }
    s + "]"
}

fn esc(c: char) -> String {
    if c.is_control() || (c as u32) > 0x7e {
        format!("\\u{{{:x}}}", c as u32)
    } else {
        c.to_string()
    }
}
