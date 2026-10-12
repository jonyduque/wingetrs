//! State of the Settings tab: which setting is selected and the values being edited.
use crate::config::{Behavior, ConfirmMode, ResidueMode};
use crate::tui::theme::GlyphSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingId {
    Confirm,
    Purge,
    KeepOld,
    Residues,
    Glyphs,
    Theme,
}

/// Order of the settings on screen.
pub const ORDER: [SettingId; 6] = [
    SettingId::Confirm,
    SettingId::Purge,
    SettingId::KeepOld,
    SettingId::Residues,
    SettingId::Glyphs,
    SettingId::Theme,
];

impl SettingId {
    pub fn label(self) -> &'static str {
        match self {
            SettingId::Confirm => "Confirmar antes de aplicar",
            SettingId::Purge => "Ao remover, apagar dados (purge)",
            SettingId::KeepOld => "Ao atualizar, manter versão antiga",
            SettingId::Residues => "Resíduos após remover",
            SettingId::Glyphs => "Glifos",
            SettingId::Theme => "Tema",
        }
    }

    pub fn options(self) -> &'static [&'static str] {
        match self {
            SettingId::Confirm => &["sempre", "só remoções", "nunca"],
            SettingId::Purge | SettingId::KeepOld => &["não", "sim"],
            SettingId::Residues => &["perguntar", "só alta confiança", "manter"],
            SettingId::Glyphs => &["nerd", "unicode", "ascii"],
            SettingId::Theme => &["escuro", "claro"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            SettingId::Confirm => {
                "Mostra o plano e espera Enter antes de instalar, atualizar ou remover."
            }
            SettingId::Purge => {
                "Apaga também os dados do app (persist) ao removê-lo. Com confirmação \"nunca\", apaga sem perguntar."
            }
            SettingId::KeepOld => "Mantém a versão anterior no disco depois de atualizar.",
            SettingId::Residues => "O que fazer com restos deixados no sistema após remover.",
            SettingId::Glyphs => "Conjunto de ícones da interface.",
            SettingId::Theme => "Paleta de cores da interface.",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub selected: usize,
    pub behavior: Behavior,
    pub glyphs: GlyphSet,
    pub dark: bool,
    /// `--glyphs` was given: the setting cannot change this session.
    pub glyphs_forced: bool,
    /// Last value written successfully.
    pub saved: (Behavior, GlyphSet, bool),
    /// The user changed Glifos this session; only then is `glyphs` written to the file.
    pub glyphs_touched: bool,
}

fn step(i: usize, len: usize, forward: bool) -> usize {
    if forward {
        (i + 1) % len
    } else {
        (i + len - 1) % len
    }
}

impl SettingsState {
    pub fn new(behavior: Behavior, glyphs: GlyphSet, dark: bool, glyphs_forced: bool) -> Self {
        SettingsState {
            selected: 0,
            behavior,
            glyphs,
            dark,
            glyphs_forced,
            saved: (behavior, glyphs, dark),
            glyphs_touched: false,
        }
    }

    pub fn current_id(&self) -> SettingId {
        ORDER[self.selected.min(ORDER.len() - 1)]
    }

    /// Index into `SettingId::options` of the current value of `id`.
    pub fn value_index(&self, id: SettingId) -> usize {
        match id {
            SettingId::Confirm => match self.behavior.confirm {
                ConfirmMode::Always => 0,
                ConfirmMode::Removals => 1,
                ConfirmMode::Never => 2,
            },
            SettingId::Purge => usize::from(self.behavior.purge),
            SettingId::KeepOld => usize::from(self.behavior.keep_old),
            SettingId::Residues => match self.behavior.residues {
                ResidueMode::Ask => 0,
                ResidueMode::High => 1,
                ResidueMode::Keep => 2,
            },
            SettingId::Glyphs => match self.glyphs {
                GlyphSet::Nerd => 0,
                GlyphSet::Unicode => 1,
                GlyphSet::Ascii => 2,
            },
            SettingId::Theme => usize::from(!self.dark),
        }
    }

    /// Cycles the selected setting; `false` (nothing changed) for Glyphs when forced by a flag.
    pub fn cycle(&mut self, forward: bool) -> bool {
        let id = self.current_id();
        if id == SettingId::Glyphs && self.glyphs_forced {
            return false;
        }
        let next = step(self.value_index(id), id.options().len(), forward);
        match id {
            SettingId::Confirm => {
                self.behavior.confirm = [
                    ConfirmMode::Always,
                    ConfirmMode::Removals,
                    ConfirmMode::Never,
                ][next];
            }
            SettingId::Purge => self.behavior.purge = next == 1,
            SettingId::KeepOld => self.behavior.keep_old = next == 1,
            SettingId::Residues => {
                self.behavior.residues =
                    [ResidueMode::Ask, ResidueMode::High, ResidueMode::Keep][next];
            }
            SettingId::Glyphs => {
                self.glyphs = [GlyphSet::Nerd, GlyphSet::Unicode, GlyphSet::Ascii][next];
            }
            SettingId::Theme => self.dark = next == 0,
        }
        true
    }
}

pub fn glyphs_name(g: GlyphSet) -> &'static str {
    match g {
        GlyphSet::Nerd => "nerd",
        GlyphSet::Unicode => "unicode",
        GlyphSet::Ascii => "ascii",
    }
}

pub fn glyphs_from_name(name: &str) -> GlyphSet {
    match name {
        "unicode" => GlyphSet::Unicode,
        "ascii" => GlyphSet::Ascii,
        _ => GlyphSet::Nerd,
    }
}
