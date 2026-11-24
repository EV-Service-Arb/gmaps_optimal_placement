#![feature(default_field_values)]
#![doc = include_str!("../README.md")]

mod held;
mod settings;

use std::{
	net::SocketAddr,
	path::{Path, PathBuf},
	sync::Arc,
};

use clap::{Parser, Subcommand, ValueEnum};
use eyre::{Result, WrapErr};
use gmaps_optimal_placement::{Base, Work};
use gmaps_optimal_placement_core::{Area, Category, Definition};
use gmaps_optimal_placement_rank::{League, Observed, Ordering, fit, pooled};
use settings::{AppConfig, SettingsCommand, SettingsFlags};

#[derive(Parser)]
#[command(about = "Paint a category's demand over the competitors already on the ground")]
struct Cli {
	#[clap(flatten)]
	settings: SettingsFlags,
	#[command(subcommand)]
	cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
	/// Refit the ranking model over every ordering in the work dir, and print the table that goes
	/// into `gmaps_optimal_placement_core::rank::COEF`. How Google ranks is one mechanism, so this takes the
	/// whole product and reads whatever each pairing has already collected
	Fit {
		#[command(flatten)]
		of: Categories,
	},
	/// Which scoring model earns the map: every entrant cross-validated over the same orderings, held
	/// out by the region each was asked from. Reads the work dir only, so it costs nothing. `fit`
	/// produces the constant; this produces the argument for which constant
	Strength {
		#[command(flatten)]
		of: Categories,
	},
	/// Which towns in a country are worth a study: something the cadastre draws, counted per
	/// administrative unit and divided by whatever the statistical grid publishes, as one HTML map
	Misc {
		country: gmaps_optimal_placement_misc::Country,
		tally: gmaps_optimal_placement_misc::Tally,
		/// Grid column to divide by — `ind` reads per person, `men` per household
		#[arg(long, default_value = "ind")]
		per: String,
		/// Skip units under this many `--per`. There is no storefront worth having in a hamlet, and
		/// what is skipped is never downloaded
		#[arg(long, default_value_t = 2000.)]
		floor: f64,
		/// Defaults to `<work dir>/out/<country>-<tally>-per-<column>.html`
		#[arg(short, long)]
		out: Option<PathBuf>,
	},
	/// The GUI: every location under every category in its country, and each location on its own,
	/// picked side by side in the page, on Google Maps (settings' `maps_key`), at 127.0.0.1
	View {
		#[command(flatten)]
		of: Categories,
		#[arg(long)]
		port: u16,
		/// Where `wasm-bindgen` put the client, as `pkg/`
		#[arg(long)]
		site: PathBuf,
	},
	/// Print a document's JSON schema. What the tool's own settings look like is `config schema` — a
	/// study is the question, the settings are what asking it costs
	Schema { doc: Doc },
	/// The tool's own settings: write defaults, diff against them, and generate the JSON Schema /
	/// Nix module an editor reads
	Config {
		#[command(subcommand)]
		cmd: SettingsCommand,
	},
}

/// Which of the conductor's categories, as the definitions it handed in carry them.
#[derive(Clone, clap::Args)]
struct Categories {
	/// A trade (`gcid:plumber`), or one setting of it (`gcid:plumber@<the first 8 of its id>`);
	/// without it, every category a definition in the work dir carries
	#[arg(short, long)]
	category: Option<String>,
}

fn main() -> Result<()> {
	color_eyre::install()?;
	let cli = Cli::parse();
	if let Cmd::Config { cmd } = cli.cmd {
		// never returns
		AppConfig::handle_settings_command(cmd, cli.settings);
	}
	let cfg = AppConfig::try_build(cli.settings)?;
	let works = {
		let (dir, cfg) = (v_utils::xdg_cache_dir!(""), cfg.clone());
		move || -> Result<Work> { Ok(Work::at(&dir, Box::new(held::Copy::open(&cfg.queries_cache)?)).policy((&cfg.age).into(), (&cfg.sweep).into())) }
	};
	let work = works()?;
	match cli.cmd {
		Cmd::Config { .. } => unreachable!("handled above, and `handle_settings_command` exits"),
		Cmd::Schema { doc } => {
			let schema = match doc {
				Doc::Base => schemars::schema_for!(Base),
			};
			println!("{}", serde_json::to_string_pretty(&schema)?);
			Ok(())
		}
		Cmd::Fit { of } => {
			let fit: fit::Fit = harvest(&of, &work)?.iter().collect();
			report(fit.stats());
			fit.check()
		}
		Cmd::Strength { of } => {
			let observed = harvest(&of, &work)?;
			let orderings: Vec<Ordering> = observed.iter().flat_map(Observed::orderings).collect();
			let league = League::run(&pooled(&observed)?, &orderings)?;
			report(league.stats());
			league.check()
		}
		Cmd::View { of, port, site } => {
			let key = cfg.maps_key.ok_or_else(|| eyre::eyre!("the GUI draws on Google Maps, and settings carry no `maps_key`"))?;
			let studied = studied(&work.path().join("studies"))?;
			let (categories, locations) = (categories(&of, &studied)?, locations(&studied));
			gmaps_optimal_placement_web::serve(SocketAddr::from(([127, 0, 0, 1], port)), site, key, categories, locations, Arc::new(works))
		}
		Cmd::Misc { country, tally, per, floor, out } => {
			let compiled = gmaps_optimal_placement_misc::compile(country, tally, &per, floor, &work)?;
			report(compiled.stats());
			write(out.unwrap_or_else(|| work.path().join("out").join(format!("{}.html", compiled.name()))), &compiled.render()?)
		}
	}
}

/// The documents a study is read from.
#[derive(Clone, Copy, ValueEnum)]
enum Doc {
	/// A country's, compiled into the library: the grid, its columns and layers.
	Base,
}

/// What every harvested study collected: every study, each once. How Google ranks is one mechanism,
/// so every one with a category contributes.
fn harvest(of: &Categories, work: &Work) -> Result<Vec<Observed>> {
	let mut observed = Vec::new();
	for def in studies(of, work)?.into_iter().filter(|d| d.category.is_some()) {
		if let Some(e) = gmaps_optimal_placement::load(&def)?.observations(work)? {
			observed.push(Observed::try_new(e.pois, e.places, &e.rankings)?);
		}
	}
	eyre::ensure!(!observed.is_empty(), "no study the conductor defined has ever been harvested");
	Ok(observed)
}

/// The two axes' product — each location under each category in its country — and every definition a
/// host ever studied here, each once.
fn studies(of: &Categories, work: &Work) -> Result<Vec<Definition>> {
	let studied = studied(&work.path().join("studies"))?;
	let categories = categories(of, &studied)?;
	let mut out: Vec<Definition> = Vec::new();
	for (_, area) in locations(&studied) {
		for (_, category) in categories.iter().filter(|(_, c)| c.country == area.country()) {
			out.push(Definition {
				area: area.clone(),
				category: Some(category.clone()),
			});
		}
	}
	for def in studied {
		if !out.iter().any(|d| d.id() == def.id()) {
			out.push(def);
		}
	}
	Ok(out)
}

/// Every area the definitions the conductor handed in are over, each once, by its slug.
fn locations(studied: &[Definition]) -> Vec<(String, Area)> {
	let mut out: Vec<(String, Area)> = Vec::new();
	for def in studied {
		if !out.iter().any(|(_, a)| *a == def.area) {
			out.push((def.area.slug().to_owned(), def.area.clone()));
		}
	}
	out.sort_by(|a, b| a.0.cmp(&b.0));
	out
}

/// Every definition a host studied in this work dir.
fn studied(dir: &Path) -> Result<Vec<Definition>> {
	// no directory is a work dir nobody has studied in yet
	if !dir.is_dir() {
		return Ok(Vec::new());
	}
	let mut out = Vec::new();
	for e in std::fs::read_dir(dir)? {
		let path = e?.path();
		if path.extension().is_some_and(|x| x == "json") {
			let text = std::fs::read_to_string(&path).wrap_err_with(|| format!("reading {}", path.display()))?;
			out.push(serde_json::from_str(&text).wrap_err_with(|| format!("parsing {}", path.display()))?);
		}
	}
	Ok(out)
}

/// Every setting of every trade the definitions the conductor handed in carry, keyed
/// `<trade name>·<CC> (<gcid>@<id8>)`. gop never reads the conductor, and a copy kept here would be
/// one more place a category drifts from it.
fn categories(of: &Categories, studied: &[Definition]) -> Result<Vec<(String, Category)>> {
	let mut keyed: Vec<(String, Category)> = Vec::new();
	for c in studied.iter().filter_map(|d| d.category.as_ref()) {
		let key = format!("{}·{} ({}@{})", c.trade.name, c.country, c.trade.id, &c.id()[..8]);
		if !keyed.iter().any(|(o, _)| *o == key) {
			keyed.push((key, c.clone()));
		}
	}
	keyed.sort_by(|a, b| a.0.cmp(&b.0));
	let Some(k) = &of.category else { return Ok(keyed) };
	let one: Vec<_> = keyed
		.iter()
		.filter(|(_, c)| *k == c.trade.id || *k == format!("{}@{}", c.trade.id, &c.id()[..8]))
		.cloned()
		.collect();
	eyre::ensure!(
		!one.is_empty(),
		"no definition in the work dir carries category {k:?}; they carry {:?}",
		keyed.iter().map(|(o, _)| o).collect::<Vec<_>>()
	);
	Ok(one)
}

fn write(out: PathBuf, html: &str) -> Result<()> {
	if let Some(dir) = out.parent() {
		std::fs::create_dir_all(dir)?;
	}
	std::fs::write(&out, html)?;
	eprintln!("wrote {} ({:.1} MB)", out.display(), html.len() as f64 / 1e6);
	Ok(())
}

fn report(stats: indexmap::IndexMap<String, String>) {
	for (k, v) in stats {
		eprintln!("  {k}: {v}");
	}
}

#[cfg(test)]
mod tests {
	use gmaps_optimal_placement_core::{Country, Excluded, Gcid, Keyword, LatLng};

	use super::*;

	/// Two settings of one trade are two categories of it, never its latest.
	#[test]
	fn a_trade_keeps_each_of_its_settings() {
		let dir = std::env::temp_dir().join(format!("gop_cli_test_settings_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let setting = |weight: f64| Definition {
			area: Area::Circle {
				slug: "clermont".into(),
				country: Country::FR,
				center: LatLng { lat: 45.78, lng: 3.08 },
				radius_m: 1000.,
				margin_m: 0.,
			},
			category: Some(Category {
				trade: Gcid {
					id: "gcid:plumber".into(),
					name: "Plombier".into(),
				},
				country: Country::FR,
				additional: Vec::new(),
				keywords: vec![
					Keyword {
						text: "plombier".into(),
						weight: Some(1.),
					},
					Keyword {
						text: "fuite".into(),
						weight: Some(weight),
					},
				],
				excluded: Excluded::default(),
				demand: None,
			}),
		};
		for d in [setting(0.5), setting(0.7), setting(0.7)] {
			std::fs::write(dir.join(format!("{}.json", d.id())), serde_json::to_vec(&d).unwrap()).unwrap();
		}
		let studied = studied(&dir).unwrap();
		std::fs::remove_dir_all(&dir).unwrap();
		let of = |category: Option<&str>| Categories {
			category: category.map(str::to_owned),
		};
		let all = categories(&of(None), &studied).unwrap();
		let under_trade = categories(&of(Some("gcid:plumber")), &studied).unwrap();
		let one = categories(&of(Some(all[0].0.split(['(', ')']).nth(1).unwrap())), &studied).unwrap();
		assert_eq!(all.len(), 2, "{all:?}");
		assert!(all.iter().all(|(k, _)| k.starts_with("Plombier·FR (gcid:plumber@")), "{all:?}");
		assert_eq!(under_trade.len(), 2);
		assert_eq!(one.len(), 1);
	}
}
