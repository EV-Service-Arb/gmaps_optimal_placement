use std::{
	net::SocketAddr,
	path::{Path, PathBuf},
	sync::Arc,
};

use clap::{Parser, Subcommand};
use eyre::Result;
use gmaps_optimal_placement::{
	Base, Study,
	settings::{AppConfig, SettingsCommand, SettingsFlags},
};
use gmaps_optimal_placement_rank::{League, Observed, Ordering, fit, pooled};
use gmaps_optimal_placement_sources::{Work, work::QueriesCache};
use sa_core::placement::{Catalog, Category, Definition};

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
	/// Serve the conductor: every request carries its assertion, checked against the keys in
	/// `PANEL_ASSERTION_KEYS`. A study is a definition it hands in, over `--countries`; nothing is
	/// bought, the conductor buys what a pull lacks
	Serve {
		#[command(flatten)]
		bases: Bases,
		#[arg(short, long, default_value_t = 8731)]
		port: u16,
	},
	/// Refit the ranking model over every ordering in the work dir, and print the table that goes
	/// into `gmaps_optimal_placement_core::rank::COEF`. How Google ranks is one mechanism, so this takes the
	/// whole product and reads whatever each pairing has already collected
	Fit {
		#[command(flatten)]
		at: Pair,
	},
	/// Which scoring model earns the map: every entrant cross-validated over the same orderings, held
	/// out by the region each was asked from. Reads the work dir only, so it costs nothing. `fit`
	/// produces the constant; this produces the argument for which constant
	Strength {
		#[command(flatten)]
		at: Pair,
	},
	/// Which towns in a country are worth a study: something the cadastre draws, counted per
	/// administrative unit and divided by whatever the statistical grid publishes, as one HTML map
	Misc {
		country: gmaps_optimal_placement::misc::Country,
		tally: gmaps_optimal_placement::misc::Tally,
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
	/// Print the study document's JSON schema. What the tool's own settings look like is `config
	/// schema` — a study is the question, the settings are what asking it costs
	Schema,
	/// The tool's own settings: write defaults, diff against them, and generate the JSON Schema /
	/// Nix module an editor reads
	Config {
		#[command(subcommand)]
		cmd: SettingsCommand,
	},
}

/// The two axes a study is a point on. The categories are the conductor's, as the definitions it
/// handed in carry them; the locations are files.
#[derive(Clone, clap::Args)]
struct Pair {
	/// One of the conductor's categories, by its key (`<name>·<CC> (trade <id>)`); without it, every
	/// category a definition in the work dir carries
	#[arg(short, long)]
	category: Option<String>,
	/// Where: the frame, the statistics office behind it, the premises in mind; a file or a
	/// directory of them
	#[arg(short, long, default_value = "examples/locations")]
	locations: PathBuf,
	#[command(flatten)]
	bases: Bases,
}

/// The country bases an area handed in at runtime stands on.
#[derive(Clone, clap::Args)]
struct Bases {
	/// `<ISO 3166-1 alpha-2>.nix` each: the grid, its columns and layers
	#[arg(long, default_value = "examples/countries")]
	countries: PathBuf,
}
impl Bases {
	/// The base an area in `def`'s country stands on.
	fn country(&self, def: &Definition) -> Result<PathBuf> {
		let path = self.countries.join(format!("{}.nix", def.area.country()));
		eyre::ensure!(path.is_file(), "no country base for {} under {}", def.area.country(), self.countries.display());
		Ok(path)
	}
}

fn main() -> Result<()> {
	color_eyre::install()?;
	let cli = Cli::parse();
	if let Cmd::Config { cmd } = cli.cmd {
		// never returns
		AppConfig::handle_settings_command(cmd, cli.settings);
	}
	let cfg = AppConfig::try_build(cli.settings)?;
	let cache: QueriesCache = cfg.queries_cache.parse()?;
	let work = Work::at(v_utils::xdg_cache_dir!(""), cache.clone()).policy((&cfg.age).into(), (&cfg.sweep).into());
	match cli.cmd {
		Cmd::Config { .. } => unreachable!("handled above, and `handle_settings_command` exits"),
		Cmd::Schema => {
			println!("{}", serde_json::to_string_pretty(&schemars::schema_for!(gmaps_optimal_placement::Study))?);
			Ok(())
		}
		Cmd::Serve { bases, port } => {
			// the work dir rather than `work` itself: `Work` holds a `Cell`, and the server calls this
			// from whichever blocking thread a request landed on
			let (dir, age, tiling) = (work.path().to_owned(), (&cfg.age).into(), (&cfg.sweep).into());
			let conductor = std::env::var("PANEL_ASSERTION_KEYS").map_err(|e| eyre::eyre!("`serve` checks every request against PANEL_ASSERTION_KEYS: {e}"))?;
			let stems = |dir: &Path| -> Result<Vec<String>> { Ok(gmaps_optimal_placement::files(dir)?.iter().map(|p| gmaps_optimal_placement::stem(p).to_owned()).collect()) };
			let catalog = Catalog {
				countries: stems(&bases.countries)?.iter().map(|c| c.parse()).collect::<Result<_, _>>()?,
			};
			let holds = {
				let bases = bases.clone();
				Arc::new(move |def: &Definition| gmaps_optimal_placement::load(def, Base::Country(&bases.country(def)?)).map(drop))
			};
			let studied = {
				let bases = bases.clone();
				move |def: &Definition, since: Option<std::time::SystemTime>| -> Result<(Study, Work)> {
					let work = Work::at(dir.clone(), cache.clone()).policy(age, tiling).since(since);
					Ok((gmaps_optimal_placement::load(def, Base::Country(&bases.country(def)?))?, work))
				}
			};
			let asks = {
				let studied = studied.clone();
				Arc::new(move |def: &Definition, since| {
					let (study, work) = studied(def, Some(since))?;
					study.asks(&work)
				})
			};
			let build = Arc::new(move |def: &Definition| {
				let (study, work) = studied(def, None)?;
				Ok(serde_json::to_string(&study.build(&work)?)?)
			});
			gmaps_optimal_placement_web::studies::serve(
				SocketAddr::from(([127, 0, 0, 1], port)),
				gmaps_optimal_placement_web::studies::Studies::new(
					conductor.parse().map_err(|e| eyre::eyre!("PANEL_ASSERTION_KEYS: {e}"))?,
					catalog,
					work.path().join("studies"),
					holds,
					asks,
					build,
				),
			)
		}
		Cmd::Fit { at } => {
			let fit: fit::Fit = harvest(&at, &work)?.iter().collect();
			report(fit.stats());
			fit.check()
		}
		Cmd::Strength { at } => {
			let observed = harvest(&at, &work)?;
			let orderings: Vec<Ordering> = observed.iter().flat_map(Observed::orderings).collect();
			let league = League::run(&pooled(&observed)?, &orderings)?;
			report(league.stats());
			league.check()
		}
		Cmd::Misc { country, tally, per, floor, out } => {
			let compiled = gmaps_optimal_placement::misc::compile(country, tally, &per, floor, &work)?;
			report(compiled.stats());
			write(out.unwrap_or_else(|| work.path().join("out").join(format!("{}.html", compiled.name()))), &compiled.render()?)
		}
	}
}

/// What every harvested study collected: the two axes' product, and every definition the conductor
/// ever handed in. How Google ranks is one mechanism, so every harvested study contributes.
fn harvest(at: &Pair, work: &Work) -> Result<Vec<Observed>> {
	let (categories, locations) = (categories(at, work)?, gmaps_optimal_placement::files(&at.locations)?);
	let mut studies = Vec::new();
	for (_, category) in &categories {
		for l in &locations {
			let area = gmaps_optimal_placement::location_area(l)?;
			if area.country() == category.country {
				let def = Definition {
					area,
					category: Some(category.clone()),
				};
				studies.push(gmaps_optimal_placement::load(&def, Base::Location(l))?);
			}
		}
	}
	let defined = work.path().join("studies");
	if defined.is_dir() {
		for e in std::fs::read_dir(&defined)? {
			let path = e?.path();
			if path.extension().is_some_and(|x| x == "json") {
				let def: Definition = read(&path)?;
				if def.category.is_some() {
					studies.push(gmaps_optimal_placement::load(&def, Base::Country(&at.bases.country(&def)?))?);
				}
			}
		}
	}
	let mut observed = Vec::new();
	for s in studies {
		observed.extend(s.observations(work)?);
	}
	eyre::ensure!(
		!observed.is_empty(),
		"no category over {}, nor any study the conductor defined, has ever been harvested",
		at.locations.display()
	);
	Ok(observed)
}

/// The latest of each of the conductor's categories, as the definitions it handed in carry them,
/// keyed. gop never reads the conductor, and a copy kept here would be one more place a category
/// drifts from it.
fn categories(at: &Pair, work: &Work) -> Result<Vec<(String, Category)>> {
	let dir = work.path().join("studies");
	let mut latest: Vec<(std::time::SystemTime, Category)> = Vec::new();
	// no directory is a work dir the conductor has never handed a study
	if dir.is_dir() {
		for e in std::fs::read_dir(&dir)? {
			let path = e?.path();
			if path.extension().is_none_or(|x| x != "json") {
				continue;
			}
			let Some(c) = read::<Definition>(&path)?.category else { continue };
			let at = std::fs::metadata(&path)?.modified()?;
			match latest.iter_mut().find(|(_, o)| (o.trade, o.country) == (c.trade, c.country)) {
				Some(slot) if slot.0 < at => *slot = (at, c),
				Some(_) => {}
				None => latest.push((at, c)),
			}
		}
	}
	let mut keyed: Vec<(String, Category)> = latest.into_iter().map(|(_, c)| (format!("{}·{} (trade {})", c.name, c.country, c.trade), c)).collect();
	keyed.sort_by(|a, b| a.0.cmp(&b.0));
	let Some(k) = &at.category else { return Ok(keyed) };
	let one: Vec<_> = keyed.iter().filter(|(o, _)| o == k).cloned().collect();
	eyre::ensure!(
		!one.is_empty(),
		"no definition under {} carries category {k:?}; they carry {:?}",
		dir.display(),
		keyed.iter().map(|(o, _)| o).collect::<Vec<_>>()
	);
	Ok(one)
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
	let text = std::fs::read_to_string(path).map_err(|e| eyre::eyre!("reading {}: {e}", path.display()))?;
	serde_json::from_str(&text).map_err(|e| eyre::eyre!("parsing {}: {e}", path.display()))
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
