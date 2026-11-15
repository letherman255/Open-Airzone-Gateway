//! Airzone domotic bus core: frames, catalogue model, rules, master engine and listener.
//! Depends on `std` only; time is passed in as `now_ms`.

pub mod api;
pub mod auth;
pub mod bus;
pub mod catalogue;
pub mod crypto;
mod catalogue_gen;
pub mod deframe;
pub mod frame;
pub mod gateway;
pub mod ha;
pub mod journal;
pub mod json;
pub mod listen;
pub mod master;
pub mod model;
pub mod poll;
pub mod registry;
pub mod rules;
pub mod settings;
pub mod topology;
pub mod update;
pub mod ws;
