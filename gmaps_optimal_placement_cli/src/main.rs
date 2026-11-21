#![feature(default_field_values)]
#![doc = include_str!("../README.md")]

mod held;
mod settings;
mod view;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use eyre::{Result, WrapErr};
use gmaps_optimal_placement::{Base, Work};
use gmaps_optimal_placement_core::{Area, Candidate, Category, Definition};
use gmaps_optimal_placement_rank::{League, Observed, Ordering, fit, pooled};
use schemars::JsonSchema;
use serde::Deserialize;
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
	/// Open a study on a map in the browser, at `127.0.0.1:<port>`, until Ctrl-C: built out of the work
	/// dir and what the conductor bought, never buying, so a study with searches missing opens with
	/// its notice. Without a study, the ones there are: `<id> <name>` a line
	View {
		/// A study's id (a prefix of it will do)
		study: Option<String>,
		#[command(flatten)]
		at: Pair,
		/// The overlay as wasm-bindgen wrote it (`nix build .#overlay`)
		#[arg(long)]
		overlay: Option<PathBuf>,
		#[arg(short, long)]
		port: Option<u16>,
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

/// The two axes a study is a point on. The categories are the conductor's, as the definitions it
/// handed in carry them; the locations are files.
#[derive(Clone, clap::Args)]
struct Pair {
	/// One of the conductor's categories, by its key (`<name>·<CC> (trade <id>)`); without it, every
	/// category a definition in the work dir carries
	#[arg(short, long)]
	category: Option<String>,
	/// Where: the area and the premises in mind; a `<slug>.toml` or a directory of them
	#[arg(short, long, default_value = "examples/locations")]
	locations: PathBuf,
}

fn main() -> Result<()> {
	color_eyre::install()?;
	let cli = Cli::parse();
	if let Cmd::Config { cmd } = cli.cmd {
		// never returns
		AppConfig::handle_settings_command(cmd, cli.settings);
	}
	let cfg = AppConfig::try_build(cli.settings)?;
	let dir = v_utils::xdg_cache_dir!("");
	let work = Work::at(&dir, Box::new(held::Copy::open(&cfg.queries_cache)?)).policy((&cfg.age).into(), (&cfg.sweep).into());
	match cli.cmd {
		Cmd::Config { .. } => unreachable!("handled above, and `handle_settings_command` exits"),
		Cmd::Schema { doc } => {
			let schema = match doc {
				Doc::Base => schemars::schema_for!(Base),
				Doc::Location => schemars::schema_for!(Location),
			};
			println!("{}", serde_json::to_string_pretty(&schema)?);
			Ok(())
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
		Cmd::View { study: None, at, .. } => {
			for (def, _) in studies(&at, &work)? {
				println!("{} {}", &def.id()[..12], def.name());
			}
			Ok(())
		}
		Cmd::View { study: Some(id), at, overlay, port } => {
			let key = cfg
				.maps_key
				.ok_or_else(|| eyre::eyre!("`view` draws on Google Maps, and needs the settings' `maps_key`: a Maps JavaScript API key"))?;
			let (overlay, port) = match (overlay, port) {
				(Some(o), Some(p)) => (o, p),
				_ => eyre::bail!("`view <study>` needs --overlay and --port; `nix run` hands it both"),
			};
			let mut found = studies(&at, &work)?.into_iter().filter(|(d, _)| d.id().starts_with(&id));
			let (Some((def, candidates)), None) = (found.next(), found.next()) else {
				eyre::bail!("{id:?} is not the id of exactly one study; `view` with none lists them")
			};
			let payload = serde_json::to_vec(&gmaps_optimal_placement::load(&def, candidates)?.build(&work)?)?;
			view::serve(payload, &overlay, &key, port)
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
	/// `--locations`' files: an area and the premises in it.
	Location,
}

/// An area and the premises worth asking about in it. A location may not name a trade.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Location {
	area: Area,
	/// A premises is a location's, not a trade's: the same unit is worth asking about for whichever
	/// trade would move into it.
	#[serde(rename = "candidate", default)]
	candidates: Vec<Candidate>,
}

/// What every harvested study collected: every study, each once. How Google ranks is one mechanism,
/// so every one with a category contributes.
fn harvest(at: &Pair, work: &Work) -> Result<Vec<Observed>> {
	let mut observed = Vec::new();
	for (def, _) in studies(at, work)?.into_iter().filter(|(d, _)| d.category.is_some()) {
		if let Some(e) = gmaps_optimal_placement::load(&def, Vec::new())?.observations(work)? {
			observed.push(Observed::try_new(e.pois, e.places, &e.rankings)?);
		}
	}
	eyre::ensure!(
		!observed.is_empty(),
		"no category over {}, nor any study the conductor defined, has ever been harvested",
		at.locations.display()
	);
	Ok(observed)
}

/// The two axes' product — each location with its premises, under each of the conductor's
/// categories in its country — and every definition a host ever studied here, each once.
fn studies(at: &Pair, work: &Work) -> Result<Vec<(Definition, Vec<Candidate>)>> {
	let mut out: Vec<(Definition, Vec<Candidate>)> = Vec::new();
	let categories = categories(at, work)?;
	for l in files(&at.locations)? {
		let Location { area, candidates } = read_toml(&l)?;
		for (_, category) in categories.iter().filter(|(_, c)| c.country == area.country()) {
			let def = Definition {
				area: area.clone(),
				category: Some(category.clone()),
			};
			out.push((def, candidates.clone()));
		}
	}
	for def in studied(work)? {
		if !out.iter().any(|(d, _)| d.id() == def.id()) {
			out.push((def, Vec::new()));
		}
	}
	Ok(out)
}

/// Every definition a host studied in this work dir.
fn studied(work: &Work) -> Result<Vec<Definition>> {
	let dir = work.path().join("studies");
	// no directory is a work dir nobody has studied in yet
	if !dir.is_dir() {
		return Ok(Vec::new());
	}
	let mut out = Vec::new();
	for e in std::fs::read_dir(&dir)? {
		let path = e?.path();
		if path.extension().is_some_and(|x| x == "json") {
			out.push(read(&path)?);
		}
	}
	Ok(out)
}

/// Every `*.toml` directly under `path`, in file-stem order. A file is itself, so one axis of the
/// product can be named on the command line while the other stays a directory.
fn files(path: &Path) -> Result<Vec<PathBuf>> {
	if path.is_file() {
		return Ok(vec![path.to_owned()]);
	}
	let mut out: Vec<PathBuf> = std::fs::read_dir(path)
		.wrap_err_with(|| format!("reading {}", path.display()))?
		.map(|e| Ok(e?.path()))
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.filter(|p| p.extension().is_some_and(|e| e == "toml"))
		.collect();
	out.sort();
	eyre::ensure!(!out.is_empty(), "no *.toml under {}", path.display());
	Ok(out)
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
	let text = std::fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
	toml::from_str(&text).wrap_err_with(|| format!("parsing {}", path.display()))
}

/// The latest of each of the conductor's categories, as the definitions it handed in carry them,
/// keyed. gop never reads the conductor, and a copy kept here would be one more place a category
/// drifts from it.
fn categories(at: &Pair, work: &Work) -> Result<Vec<(String, Category)>> {
	let dir = work.path().join("studies");
	let mut latest: Vec<(std::time::SystemTime, Category)> = Vec::new();
	// no directory is a work dir nobody has studied in yet
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
