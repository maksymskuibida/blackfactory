//! `bf space`: named in this version, built by a later unit (placed by unit U-1). It answers with one refusal.

use super::{not_built, Args};
use crate::refusal::R;

pub(crate) fn cmd_space(_args: &Args) -> R<()> {
    Err(not_built("space"))
}
