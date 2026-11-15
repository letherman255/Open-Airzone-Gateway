//! Types of the register catalogue. The table itself is generated from `catalogue/registers.toml`
//! into `catalogue_gen.rs` (run `python -m generator registers.toml --out ..` in `catalogue/`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    System,
    Zone,
    Iaq,
    Outlet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Enum,
    Bool,
    Text,
    /// One-shot command: writing sets its bits to 1.
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    Confirmed,
    Matched,
    Hypothesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requires {
    IaqPresent,
    VmcOn,
    Ionizer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Danger {
    pub level: &'static str,
    pub warning: &'static str,
    pub confirm: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    pub id: &'static str,
    pub scope: Scope,
    /// Offset in the zone / outlet block, absolute for system and IAQ.
    pub reg: u16,
    pub lo: u8,
    pub hi: u8,
    pub kind: Kind,
    pub scale: f32,
    pub offset: f32,
    pub unit: &'static str,
    pub values: &'static [(u16, &'static str)],
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// Registers (text only).
    pub length: u8,
    pub writable: bool,
    pub mask_write: bool,
    pub tier: u8,
    pub confidence: Confidence,
    pub requires: Option<Requires>,
    pub danger: Option<Danger>,
    /// Home Assistant component and role from the catalogue `ha` block ("" when absent).
    pub ha_component: &'static str,
    pub ha_role: &'static str,
    /// Human label (catalogue `ui.label`).
    pub label: &'static str,
}

pub use crate::catalogue_gen::PARAMS;

/// Catalogue entry by id.
pub fn param(id: &str) -> Option<&'static Param> {
    PARAMS.binary_search_by(|p| p.id.cmp(id)).ok().map(|i| &PARAMS[i])
}
