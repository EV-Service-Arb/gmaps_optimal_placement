use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use eyre::Result;
use gmaps_optimal_placement::{
	Base, Study,
	core::define::{Category, Definition},
	settings::{AppConfig, SettingsCommand, SettingsFlags},
};
use gmaps_optimal_placement_rank::{League, Observed, Ordering, fit, pooled};
use gmaps_optimal_placement_sources::{Work, work::QueriesCache};

#[derive(Parser)]
#[command(about = "Paint a study's demand model over the competitors already on the ground")]
struct Cli {
	#[clap(flatten)]
	settings: SettingsFlags,
	/// Buy what the work dir lacks, on real Google quota. Without it Places answers come out of the
	/// work dir or not at all: a map missing any of its inventory opens on what the work dir holds and
	/// says how much is missing, and a probe stops at its first miss
	#[arg(long, global = true)]
	spend_real_quota_i_am_certain: bool,
	#[command(subcommand)]
	cmd: Cmd,
}

/// The two axes a study is a point on, and the expert half each category names. Named rather than
/// positional: the axes are the same shape, so nothing about a bare pair of paths says which one is
/// which. Either may be a single file instead of a directory, which is how a scripted run names one
/// without meeting a picker.
#[derive(Clone, clap::Args)]
struct Pair {
	/// What is being sold, as the conductor holds it: a trade in a country, its keywords and Google
	/// categories, and the trade file that studies it (`*.json`)
	#[arg(short, long, default_value = "examples/categories")]
	categories: PathBuf,
	/// Where: the frame, the statistics office behind it, the premises in mind
	#[arg(short, long, default_value = "examples/locations")]
	locations: PathBuf,
	#[command(flatten)]
	experts: Experts,
}

/// What only an expert writes, looked up by name: the trades a category's `study` names, and the
/// country bases an area handed in at runtime stands on.
#[derive(Clone, clap::Args)]
struct Experts {
	/// `<study>.nix` each: the demand model, the tier vocabulary, λ
	#[arg(short, long, default_value = "examples/trades")]
	trades: PathBuf,
	/// `<ISO 3166-1 alpha-2>.nix` each: the grid, its columns and layers, the volume provider
	#[arg(long, default_value = "examples/countries")]
	countries: PathBuf,
}

#[derive(Subcommand)]
enum Cmd {
	/// Where the people are: demand under the competitors, as a map served from here. The page picks
	/// a category and a city out of the two directories, a field per axis; naming both as files opens
	/// the one tab, already built. The category field also offers none, which serves the city off the
	/// statistical archive alone and spends nothing
	Serve {
		#[command(flatten)]
		at: Pair,
		#[arg(short, long, default_value_t = 8731)]
		port: u16,
		/// Point the desktop browser at it once it is up
		#[arg(long)]
		open: bool,
		/// Buy the competitor inventory again rather than serve what is on disk. The only thing that
		/// spends on a question already answered — an answer past its `age` is reported, never refetched
		#[arg(long)]
		refresh: bool,
		/// Serve the conductor instead of a page: every request but `/mfe/*` carries its assertion,
		/// checked against the keys in `PANEL_ASSERTION_KEYS`, and only a pull spends. Studies are the
		/// definitions it hands in, over `--trades` and `--countries`; the two axes are not read
		#[arg(long, conflicts_with_all = ["open", "refresh"])]
		behind_conductor: bool,
	},
	/// How many people ask for it: monthly volume per query group, as one HTML chart
	Searches {
		#[command(flatten)]
		at: Pair,
		/// Defaults to `<work dir>/out/<study name>-searches.html`
		#[arg(short, long)]
		out: Option<PathBuf>,
	},
	/// Ask Google the study's terms from equal-demand nodes, so the ranking model has orderings a
	/// searcher could have produced
	Probe {
		#[command(flatten)]
		at: Pair,
		/// Print the node placement and the call budget, spend nothing
		#[arg(long)]
		dry_run: bool,
		/// Ask every node again rather than serve the orderings on disk
		#[arg(long)]
		refresh: bool,
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

fn main() -> Result<()> {
	color_eyre::install()?;
	let cli = Cli::parse();
	if let Cmd::Config { cmd } = cli.cmd {
		// never returns
		AppConfig::handle_settings_command(cmd, cli.settings);
	}
	let cfg = AppConfig::try_build(cli.settings)?;
	let refresh = matches!(cli.cmd, Cmd::Serve { refresh, .. } | Cmd::Probe { refresh, .. } if refresh);
	let spend = cli.spend_real_quota_i_am_certain;
	eyre::ensure!(!refresh || spend, "`--refresh` buys every answer again, which only `--spend-real-quota-i-am-certain` does");
	// behind the conductor a GET never spends: "Pull latest", a POST, is the one way to buy
	let behind = matches!(cli.cmd, Cmd::Serve { behind_conductor: true, .. });
	let keys: Vec<(String, String)> = (&cfg).into();
	let cache: QueriesCache = cfg.queries_cache.parse()?;
	let work = Work::at(v_utils::xdg_cache_dir!(""), cache.clone())
		.policy((&cfg.age).into(), (&cfg.sweep).into())
		.keys(keys.clone())
		.refresh(refresh.then(std::time::SystemTime::now))
		.spend(spend && !behind);
	match cli.cmd {
		Cmd::Config { .. } => unreachable!("handled above, and `handle_settings_command` exits"),
		Cmd::Schema => {
			println!("{}", serde_json::to_string_pretty(&schemars::schema_for!(gmaps_optimal_placement::Study))?);
			Ok(())
		}
		Cmd::Serve {
			at,
			port,
			open,
			refresh: _,
			behind_conductor: _,
		} => {
			let addr = SocketAddr::from(([127, 0, 0, 1], port));
			// the work dir rather than `work` itself: `Work` holds a `Cell`, and the server calls this
			// from whichever blocking thread a request landed on. `--refresh` does not come along: it
			// belongs to the run that asked for it, and a tab switch is not one — the page's own reload
			// is. Spending does, because it is about the server and not about a run
			let (dir, age, tiling) = (work.path().to_owned(), (&cfg.age).into(), (&cfg.sweep).into());
			let worker = {
				let (keys, cache) = (keys.clone(), cache.clone());
				move |refresh: Option<std::time::SystemTime>| -> Result<Work> {
					eyre::ensure!(refresh.is_none() || spend, "this server buys nothing: it was started without `--spend-real-quota-i-am-certain`");
					Ok(Work::at(dir.clone(), cache.clone())
						.policy(age, tiling)
						.keys(keys.clone())
						.refresh(refresh)
						.spend(spend && refresh.is_some()))
				}
			};
			let experts = at.experts.clone();
			if behind {
				let keys = std::env::var("PANEL_ASSERTION_KEYS").map_err(|e| eyre::eyre!("`--behind-conductor` checks every request against PANEL_ASSERTION_KEYS: {e}"))?;
				let stems = |dir: &std::path::Path| -> Result<Vec<String>> { Ok(gmaps_optimal_placement::files(dir)?.iter().map(|p| gmaps_optimal_placement::stem(p).to_owned()).collect()) };
				let catalog = serde_json::json!({"trades": stems(&experts.trades)?, "countries": stems(&experts.countries)?}).to_string();
				let holds = {
					let experts = experts.clone();
					std::sync::Arc::new(move |def: &Definition| gmaps_optimal_placement::load(def, Base::Country(&experts.country(def)?), &experts.trades).map(drop))
				};
				let build = std::sync::Arc::new(move |def: &Definition, pull: Option<gmaps_optimal_placement_web::serve::Pull>| -> (usize, Result<String>) {
					let work = match worker(pull.as_ref().map(|p| p.since)) {
						Ok(w) => w.budget(pull.map(|p| p.budget)),
						Err(e) => return (0, Err(e)),
					};
					let built = (|| {
						Ok(serde_json::to_string(
							&gmaps_optimal_placement::load(def, Base::Country(&experts.country(def)?), &experts.trades)?.build(&work)?,
						)?)
					})();
					(work.billed(), built)
				});
				return gmaps_optimal_placement_web::serve::serve(
					addr,
					gmaps_optimal_placement_web::serve::Mode::Conductor {
						keys: keys.parse().map_err(|e| eyre::eyre!("PANEL_ASSERTION_KEYS: {e}"))?,
						catalog,
						studies: work.path().join("studies"),
						holds,
						build,
					},
				);
			}
			// the page picks out of the two directories, so `fzf` is not one of two pickers to learn. A
			// pair named as files is built here instead: its statistics print, and a study that will not
			// build fails before the listener binds
			let (categories, locations) = (gmaps_optimal_placement::files(&at.categories)?, gmaps_optimal_placement::files(&at.locations)?);
			let prebuilt = match (categories.as_slice(), locations.as_slice()) {
				([category], [location]) => {
					let payload = study(Some(category), location, &experts)?.build(&work)?;
					report(gmaps_optimal_placement::stats(&payload));
					vec![(
						Some(gmaps_optimal_placement::stem(category).to_owned()),
						gmaps_optimal_placement::stem(location).to_owned(),
						serde_json::to_string(&payload)?,
					)]
				}
				_ => Vec::new(),
			};
			let located = locations
				.iter()
				.map(|l| {
					let s = study(None, l, &experts)?;
					let b = s.area.frame().bbox;
					Ok(gmaps_optimal_placement_web::serve::Location {
						path: l.clone(),
						bbox: [b.lon[0], b.lat[0], b.lon[1], b.lat[1]],
						country: s.country,
					})
				})
				.collect::<Result<Vec<_>>>()?;
			let key = cfg
				.google
				.iter()
				.find_map(|a| a.provides.browser.as_ref())
				.ok_or_else(|| eyre::eyre!("the map loads Google's script with a `browser` key, and no account in the config's `google` provides one"))?
				.key
				.clone();
			let build = std::sync::Arc::new({
				let (experts, worker) = (experts.clone(), worker.clone());
				move |category: Option<&std::path::Path>, location: &std::path::Path, refresh: bool| {
					// gop's own page buys on a GET, as it always has; only behind the conductor does a GET read alone
					let work = match refresh {
						true => worker(Some(std::time::SystemTime::now()))?,
						false => worker(None)?.spend(spend),
					};
					let payload = study(category, location, &experts)?.build(&work)?;
					Ok(serde_json::to_string(&payload)?)
				}
			});
			let find = std::sync::Arc::new(move |location: &std::path::Path, q: &str| {
				let frame = study(None, location, &experts)?.area.frame().bbox;
				gmaps_optimal_placement_sources::poi::find(&worker(None)?.spend(spend), frame, q)
			});
			gmaps_optimal_placement_web::serve::serve(
				addr,
				gmaps_optimal_placement_web::serve::Mode::Page {
					key,
					open,
					categories,
					locations: located,
					prebuilt,
					build,
					find,
				},
			)
		}
		Cmd::Probe { at, dry_run, refresh: _ } => {
			let (category, location) = (gmaps_optimal_placement::pick(&at.categories)?, gmaps_optimal_placement::pick(&at.locations)?);
			study(Some(&category), &location, &at.experts)?.probe(&work, dry_run)?;
			Ok(())
		}
		Cmd::Fit { at } => {
			let (studies, observed) = harvest(&at, &work)?;
			// `harvest` only keeps a study that contributed orderings, and only a trade can
			let lambda: Vec<f64> = studies.iter().map(|s| s.model.as_ref().expect("a study with orderings has a trade").lambda_m).collect();
			let fit: fit::Fit = observed.iter().collect();
			report(fit.stats(&lambda));
			fit.check()
		}
		Cmd::Strength { at } => {
			let (_, observed) = harvest(&at, &work)?;
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
		Cmd::Searches { at, out } => {
			let (category, location) = (gmaps_optimal_placement::pick(&at.categories)?, gmaps_optimal_placement::pick(&at.locations)?);
			let payload = study(Some(&category), &location, &at.experts)?.searches(&work, &cfg.google.iter().filter_map(|a| a.provides.ads.as_ref().map(Into::into)).collect::<Vec<_>>())?;
			report(gmaps_optimal_placement::search_stats(&payload));
			let html = gmaps_optimal_placement::render::render_searches(&payload)?;
			write(out.unwrap_or_else(|| work.path().join("out").join(format!("{}-searches.html", payload.name))), &html)
		}
	}
}

/// Every study that has been harvested, and what it collected: the two axes' product, and every
/// definition the conductor ever handed in. No picker: how Google ranks is one mechanism, so every
/// harvested study contributes. The studies come back alongside, filtered to the ones that said
/// something.
fn harvest(at: &Pair, work: &Work) -> Result<(Vec<Study>, Vec<Observed>)> {
	let (categories, locations) = (gmaps_optimal_placement::files(&at.categories)?, gmaps_optimal_placement::files(&at.locations)?);
	let mut studies = Vec::new();
	for c in &categories {
		let category: Category = read(c)?;
		for l in &locations {
			let area = gmaps_optimal_placement::location_area(l)?;
			if area.country() == category.country {
				let def = Definition {
					area,
					category: Some(category.clone()),
				};
				studies.push(gmaps_optimal_placement::load(&def, Base::Location(l), &at.experts.trades)?);
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
					studies.push(gmaps_optimal_placement::load(&def, Base::Country(&at.experts.country(&def)?), &at.experts.trades)?);
				}
			}
		}
	}
	let (mut kept, mut observed) = (Vec::new(), Vec::new());
	for s in studies {
		if let Some(o) = s.observations(work)? {
			kept.push(s);
			observed.push(o);
		}
	}
	eyre::ensure!(
		!observed.is_empty(),
		"nothing under {} × {}, nor any study the conductor defined, has ever been harvested",
		at.categories.display(),
		at.locations.display()
	);
	Ok((kept, observed))
}

/// A category file over a location file, or the location alone.
fn study(category: Option<&std::path::Path>, location: &std::path::Path, experts: &Experts) -> Result<Study> {
	let def = Definition {
		area: gmaps_optimal_placement::location_area(location)?,
		category: category.map(read).transpose()?,
	};
	gmaps_optimal_placement::load(&def, Base::Location(location), &experts.trades)
}

impl Experts {
	/// The base an area in `def`'s country stands on.
	fn country(&self, def: &Definition) -> Result<PathBuf> {
		let path = self.countries.join(format!("{}.nix", def.area.country()));
		eyre::ensure!(path.is_file(), "no country base for {} under {}", def.area.country(), self.countries.display());
		Ok(path)
	}
}

fn read<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
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
