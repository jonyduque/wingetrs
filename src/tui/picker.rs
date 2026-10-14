//! Pure state of the arrow-key picker, shared by the CLI (inline) and the TUI.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    pub label: String,
    pub detail: String,
    pub selectable: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickOutcome {
    Pending,
    Picked(Vec<usize>),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Picker {
    items: Vec<PickerItem>,
    selected: usize,
    checked: Vec<bool>,
    multi: bool,
}

impl Picker {
    pub fn single(items: Vec<PickerItem>) -> Picker {
        let n = items.len();
        Picker::build(items, vec![false; n], false)
    }

    /// Single mode with the cursor on `at`, even when that item is disabled.
    pub fn single_at(items: Vec<PickerItem>, at: usize) -> Picker {
        let mut p = Picker::single(items);
        p.selected = at.min(p.items.len().saturating_sub(1));
        p
    }

    /// Multiple mode: `initial` pre-checks rows (missing entries count as unchecked).
    pub fn multi(items: Vec<PickerItem>, initial: &[bool]) -> Picker {
        let checked = items
            .iter()
            .enumerate()
            .map(|(i, it)| initial.get(i).copied().unwrap_or(false) && it.selectable)
            .collect();
        Picker::build(items, checked, true)
    }

    fn build(items: Vec<PickerItem>, checked: Vec<bool>, multi: bool) -> Picker {
        let selected = items.iter().position(|i| i.selectable).unwrap_or(0);
        Picker {
            items,
            selected,
            checked,
            multi,
        }
    }

    pub fn items(&self) -> &[PickerItem] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn checked(&self) -> &[bool] {
        &self.checked
    }

    pub fn is_multi(&self) -> bool {
        self.multi
    }

    fn step(&mut self, forward: bool) {
        let next = if forward {
            (self.selected + 1..self.items.len()).find(|&i| self.items[i].selectable)
        } else {
            (0..self.selected).rev().find(|&i| self.items[i].selectable)
        };
        if let Some(i) = next {
            self.selected = i;
        }
    }

    pub fn key(&mut self, ev: KeyEvent) -> PickOutcome {
        if ev.kind == KeyEventKind::Release {
            return PickOutcome::Pending;
        }
        match ev.code {
            KeyCode::Char('c') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                return PickOutcome::Cancelled;
            }
            KeyCode::Down => self.step(true),
            KeyCode::Up => self.step(false),
            KeyCode::Home => {
                if let Some(i) = self.items.iter().position(|i| i.selectable) {
                    self.selected = i;
                }
            }
            KeyCode::End => {
                if let Some(i) = self.items.iter().rposition(|i| i.selectable) {
                    self.selected = i;
                }
            }
            KeyCode::Char(' ') if self.multi => {
                if self.items.get(self.selected).is_some_and(|i| i.selectable) {
                    self.checked[self.selected] = !self.checked[self.selected];
                }
            }
            KeyCode::Esc => return PickOutcome::Cancelled,
            KeyCode::Enter => {
                if self.multi {
                    let v = (0..self.items.len()).filter(|&i| self.checked[i]).collect();
                    return PickOutcome::Picked(v);
                }
                if self.items.get(self.selected).is_some_and(|i| i.selectable) {
                    return PickOutcome::Picked(vec![self.selected]);
                }
            }
            _ => {}
        }
        PickOutcome::Pending
    }
}

/// Visible slice of `len` rows in `height` lines that keeps `selected` in view.
pub fn window(selected: usize, len: usize, height: usize) -> Range<usize> {
    let height = height.max(1);
    if len <= height {
        return 0..len;
    }
    let start = (selected + 1).saturating_sub(height).min(len - height);
    start..start + height
}
