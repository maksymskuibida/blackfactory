//! Black Factory engine library. The `bf` binary is a thin wrapper over `cli::main_with`.

pub mod cli;
pub mod doctor;
pub mod events;
pub mod evidence;
pub mod gates;
pub mod gitx;
pub mod guard;
pub mod ledger;
pub mod plan;
pub mod refusal;
pub mod run;
pub mod sandbox;
pub mod schema;
pub mod store;
pub mod util;
pub mod view;
