//! CTAO observatory inference and telescope resource construction.

use siderust::calculus::solar::Twilight;
use siderust::observatories::{EL_PARANAL, ROQUE_DE_LOS_MUCHACHOS};
use std::path::{Path, PathBuf};

use super::model::{
    OutLocation, OutMoonAltitudeConstraint, OutNightTimeConstraint, OutResourceHardConstraints,
    OutTelescope,
};

const DEFAULT_TELESCOPE_NIGHT_TWILIGHT: Twilight = Twilight::Nautical;
const DEFAULT_MOON_ALTITUDE_MIN_DEG: f64 = -90.0;
const DEFAULT_MOON_ALTITUDE_MAX_DEG: f64 = 0.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CtaoObservatory {
    North,
    South,
}

impl CtaoObservatory {
    pub(crate) fn code(self) -> &'static str {
        match self {
            CtaoObservatory::North => "CTA-N",
            CtaoObservatory::South => "CTA-S",
        }
    }

    fn telescope_id(self) -> u64 {
        match self {
            CtaoObservatory::North => 0,
            CtaoObservatory::South => 1,
        }
    }

    pub(crate) fn telescope_resource(self) -> OutTelescope {
        let site = match self {
            CtaoObservatory::North => ROQUE_DE_LOS_MUCHACHOS,
            CtaoObservatory::South => EL_PARANAL,
        };

        OutTelescope {
            id: self.telescope_id(),
            name: self.code().to_string(),
            location: OutLocation {
                longitude_deg: site.lon.value(),
                latitude_deg: site.lat.value(),
                height_m: site.height.value(),
            },
            hard_constraints: OutResourceHardConstraints {
                night_time: OutNightTimeConstraint {
                    twilight: twilight_schema_name(DEFAULT_TELESCOPE_NIGHT_TWILIGHT).to_string(),
                },
                moon_altitude: OutMoonAltitudeConstraint {
                    min_deg: DEFAULT_MOON_ALTITUDE_MIN_DEG,
                    max_deg: DEFAULT_MOON_ALTITUDE_MAX_DEG,
                },
            },
        }
    }
}

pub(crate) fn infer_observatory(
    dataset_dir: &Path,
    json_files: &[PathBuf],
) -> Result<CtaoObservatory, String> {
    let mut saw_north = false;
    let mut saw_south = false;
    let mut saw_lst_cycle = false;

    let dir = dataset_dir.to_string_lossy().to_ascii_uppercase();
    if dir.contains("CTA-N") || dir.contains("CTA_N") {
        saw_north = true;
    }
    if dir.contains("CTA-S") || dir.contains("CTA_S") {
        saw_south = true;
    }
    if dir.contains("LST") {
        saw_lst_cycle = true;
    }

    for path in json_files {
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let name = file_name.to_ascii_uppercase();
        if name.contains("_N_") || name.contains("CTA-N") || name.contains("CTA_N") {
            saw_north = true;
        }
        if name.contains("_S_") || name.contains("CTA-S") || name.contains("CTA_S") {
            saw_south = true;
        }
        if name.contains("LST") {
            saw_lst_cycle = true;
        }
    }

    match (saw_north, saw_south) {
        (true, false) => Ok(CtaoObservatory::North),
        (false, true) => Ok(CtaoObservatory::South),
        (false, false) if saw_lst_cycle => Ok(CtaoObservatory::North),
        (true, true) => Err(format!(
            "cannot infer a single observatory from {}: both CTA-N and CTA-S markers were found",
            dataset_dir.display()
        )),
        (false, false) => Err(format!(
            "cannot infer observatory from {}: expected CTA-N/CTA-S in directory or file names",
            dataset_dir.display()
        )),
    }
}

fn twilight_schema_name(twilight: Twilight) -> &'static str {
    match twilight {
        Twilight::Civil => "Civil",
        Twilight::Nautical => "Nautical",
        Twilight::Astronomical => "Astronomical",
        Twilight::Horizon => "Horizon",
        Twilight::ApparentHorizon => "ApparentHorizon",
    }
}
