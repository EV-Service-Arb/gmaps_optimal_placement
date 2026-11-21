//! A work dir a host's tests build a real study in, offline: the Filosofi rows and the commune names
//! of central Clermont-Ferrand, under the archive's own names.
use std::{io::Write, path::Path};

use eyre::Result;
use gmaps_optimal_placement_core::LatLng;

/// The fixture's cells cover a circle of [`RADIUS_M`] around this.
pub const CENTER: LatLng = LatLng { lat: 45.78, lng: 3.07 };
pub const RADIUS_M: f64 = 800.;

pub fn fixture(dir: &Path) -> Result<()> {
	let data = dir.join("data");
	std::fs::create_dir_all(&data)?;
	let mut z = zip::ZipWriter::new(std::fs::File::create(data.join("Filosofi2021_carreaux_200m_csv.zip"))?);
	z.start_file("carreaux_200m_met.csv", zip::write::SimpleFileOptions::default())?;
	z.write_all(include_bytes!("testing/carreaux_200m_met.csv"))?;
	z.finish()?;
	std::fs::write(data.join("communes_63.json"), include_str!("testing/communes_63.json"))?;
	Ok(())
}
