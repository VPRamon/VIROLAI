//! User-facing EST figure-of-merit selector.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use crate::scheduler::fom::{ScheduleFom, SoftConstraintFom};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum EstFomKind {
    #[default]
    SoftConstraint,
}

impl EstFomKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SoftConstraint => "soft_constraint",
        }
    }

    pub fn into_fom(self) -> Arc<dyn ScheduleFom> {
        match self {
            Self::SoftConstraint => Arc::new(SoftConstraintFom),
        }
    }
}

impl fmt::Display for EstFomKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EstFomKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "soft_constraint" => Ok(Self::SoftConstraint),
            other => Err(format!(
                "invalid EST FOM '{other}' (expected 'soft_constraint')"
            )),
        }
    }
}
