//! The GEOSTAT arm end to end, on four real rows of the Eurostat census grid.
//!
//! The archive is 566 MB, so the test writes one with the same member name and header instead. What
//! it checks is everything downstream of the download: the 1 km id, the reprojection, and which
//! columns are text.
use std::io::Write;

use gmaps_optimal_placement_core::grid::Bbox;
use gmaps_optimal_placement_sources::{GridSource, Work, grid, work::Store};

const HEADER: &str = "GRD_ID,T,M,F,Y_LT15,Y_1564,Y_GE65,EMP,NAT,EU_OTH,OTH,SAME,CHG_IN,CHG_OUT,LAND_SURFACE,POPULATED,CNTR_ID";
const ROWS: &[&str] = &[
	"CRS3035RES1000mN2683000E4285000,0,0,0,0,0,0,0,0,0,0,0,0,0,0.9471130000000001,0,AT-CH-LI",
	"CRS3035RES1000mN2684000E4285000,25,14,12,6,18,3,2,22,0,1,23,0,0,0.903667,1,AT-CH-LI",
	"CRS3035RES1000mN2683000E4286000,116,63,53,19,79,18,54,96,9,11,106,7,0,0.9899,1,AT-LI",
	"CRS3035RES1000mN2685000E4286000,439,224,213,76,293,70,0,367,47,24,417,15,3,0.851269,1,AT-CH",
];

fn work_with_fixture(name: &str) -> (tempdir::Dir, Work) {
	let dir = tempdir::Dir::new(name);
	std::fs::create_dir_all(dir.path().join("data")).unwrap();
	let f = std::fs::File::create(dir.path().join("data/Eurostat_Census-GRID_2021_V3.zip")).unwrap();
	let mut z = zip::ZipWriter::new(f);
	z.start_file("Eurostat_Census-GRID_2021_V3/ESTAT_Census_2021_V3.csv", zip::write::SimpleFileOptions::default())
		.unwrap();
	writeln!(z, "{HEADER}").unwrap();
	for r in ROWS {
		writeln!(z, "{r}").unwrap();
	}
	z.finish().unwrap();
	let work = Work::at(dir.path(), Store::Alone);
	(dir, work)
}

#[test]
fn four_cells_of_the_alps() {
	let (_dir, work) = work_with_fixture("alps");
	let bbox = Bbox { lat: [47.0, 47.3], lon: [9.4, 9.8] };
	let g = grid::load(GridSource::Geostat1km, 2021, bbox, &work).unwrap();

	assert_eq!(g.len(), 4, "every fixture row is inside the bbox");
	assert_eq!(g.columns.len(), 15, "GRD_ID and CNTR_ID are text, the other 15 are not");
	assert_eq!(g.columns["T"], vec![0., 25., 116., 439.]);
	assert_eq!(g.columns["LAND_SURFACE"][0], 0.9471130000000001);
	assert_eq!(g.cells[0].place, "AT-CH-LI");
	assert!(g.cells.iter().all(|c| !c.imputed), "GEOSTAT publishes no provenance flag");

	// the row says AT-CH-LI, and that tripoint is at 9.52 E 47.26 N — the id and the reprojection
	// agree with a fact neither of them carries
	let sw = g.cells[0].ring[0];
	assert!(
		(sw[0] - 9.5247167).abs() < 1e-5 && (sw[1] - 47.2595447).abs() < 1e-5,
		"1 km cell N2683000 E4285000 lands at {sw:?}"
	);
	// the ring is one kilometre across, not one metre and not one degree
	let width_deg = g.cells[0].ring[1][0] - g.cells[0].ring[0][0];
	assert!((width_deg * 111_320. * 47f64.to_radians().cos() - 1000.).abs() < 20., "east edge spans {width_deg} deg");
}

/// The rows kept beside the archive stand in for a scan of it, so what comes back has to be the same
/// grid to the bit. A cell that moved by an ulp is the shifted-grid bug with a smaller number on it,
/// and the JSON that would have carried floats here does not round-trip one.
#[test]
fn what_is_kept_rebuilds_the_grid_exactly() {
	let (dir, work) = work_with_fixture("extract");
	let bbox = Bbox { lat: [47.0, 47.3], lon: [9.4, 9.8] };

	let scanned = grid::load(GridSource::Geostat1km, 2021, bbox, &work).unwrap();
	let kept: Vec<_> = std::fs::read_dir(dir.path().join("data/extract")).unwrap().map(|e| e.unwrap().file_name()).collect();
	assert_eq!(kept.len(), 1, "the scan left the rows it kept behind: {kept:?}");

	let again = grid::load(GridSource::Geostat1km, 2021, bbox, &work).unwrap();
	assert_eq!(again.len(), scanned.len());
	for (a, b) in scanned.cells.iter().zip(&again.cells) {
		assert_eq!((&a.id, &a.place, a.imputed), (&b.id, &b.place, b.imputed));
		let bits = |r: &[[f64; 2]; 4]| r.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>();
		assert_eq!(bits(&a.ring), bits(&b.ring), "cell {} came back on a different ring", a.id);
	}
	for (name, values) in &scanned.columns {
		let back = &again.columns[name];
		let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
		assert_eq!(bits(values), bits(back), "column {name:?} came back with different numbers");
	}

	// the frame is half the key: a narrower one is a different question and not a hit on this answer
	let narrower = Bbox {
		lat: [47.0, 47.27],
		lon: [9.4, 9.8],
	};
	let fewer = grid::load(GridSource::Geostat1km, 2021, narrower, &work).unwrap();
	assert!(fewer.len() < scanned.len(), "{} cells for a narrower frame, against {}", fewer.len(), scanned.len());
	assert_eq!(std::fs::read_dir(dir.path().join("data/extract")).unwrap().count(), 2, "and it is kept under its own key");
}

/// A vintage with no archive behind it is an error, not an empty map.
#[test]
fn unknown_vintage() {
	let (_dir, work) = work_with_fixture("vintage");
	let bbox = Bbox { lat: [47.0, 47.3], lon: [9.4, 9.8] };
	let e = grid::load(GridSource::Geostat1km, 2011, bbox, &work).unwrap_err().to_string();
	assert!(e.contains("2011"), "{e}");
}

mod tempdir {
	use std::path::{Path, PathBuf};

	pub struct Dir(PathBuf);

	impl Dir {
		pub fn new(name: &str) -> Self {
			let p = std::env::temp_dir().join(format!("gmaps_optimal_placement_test_{name}_{}", std::process::id()));
			let _ = std::fs::remove_dir_all(&p);
			std::fs::create_dir_all(&p).unwrap();
			Self(p)
		}

		pub fn path(&self) -> &Path {
			&self.0
		}
	}

	impl Drop for Dir {
		fn drop(&mut self) {
			let _ = std::fs::remove_dir_all(&self.0); // best effort: a leftover temp dir is not a test failure
		}
	}
}
