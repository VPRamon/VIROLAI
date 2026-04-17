//! Serde support for [`Telescope`](super::Telescope).

use super::Telescope;
use crate::serde_repr::{
    HardConstraintsRepr, LocationRepr, geodetic_location_from_repr,
    hard_constraint_blocks_from_repr,
};
use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub(crate) struct TelescopeRepr {
    pub id: u64,
    #[serde(default)]
    pub name: Option<String>,
    pub location: LocationRepr,
    #[serde(default)]
    pub hard_constraints: HardConstraintsRepr,
}

impl TelescopeRepr {
    pub(crate) fn into_telescope(self) -> Result<Telescope, String> {
        let location = geodetic_location_from_repr(self.location)?;
        let hard_constraints = hard_constraint_blocks_from_repr(&self.hard_constraints)?;
        let name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("telescope-{}", self.id));

        Ok(Telescope {
            id: self.id,
            name,
            location,
            hard_constraints,
        })
    }
}

impl<'de> Deserialize<'de> for Telescope {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = TelescopeRepr::deserialize(deserializer)?;
        repr.into_telescope().map_err(serde::de::Error::custom)
    }
}
