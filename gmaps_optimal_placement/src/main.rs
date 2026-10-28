use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};
use eyre::Result;
use gmaps_optimal_placement::{
	Study,
	settings::{AppConfig, SettingsCommand, SettingsFlags},
};
use gmaps_optimal_placement_rank::{League, Observed, Ordering, fit, pooled};
use gmaps_optimal_placement_sources::Work;

#[derive(Parser)]
#[command(about = "Paint a study's demand model over the competitors already on the ground")]
struct Cli {
	#[clap(flatten)]
	settings: SettingsFlags,
	/// Spend no Google Maps credit: Places answers come out of the work dir or not at all. A map
	/// missing any of its inventory opens on demand alone, and a probe stops at its first miss
	#[arg(long, global = true)]
	block_spend: bool,
	#[command(subcommand)]
	cmd: Cmd,
}

/// The two axes a study is a point on. Named rather than positional: the two are the same shape, so
/// nothing about a bare pair of paths says which one is which. Either may be a single `*.nix`
/// instead of a directory, which is how a scripted run names one without meeting a picker.
#[derive(Clone, clap::Args)]
struct Pair {
	/// What is being sold: queries, tiering, the demand model, the ranking terms
	#[arg(short, long, default_value = "examples/trades")]
	trades: PathBuf,
	/// Where: the frame, the statistics office behind it, the premises in mind
	#[arg(short, long, default_value = "examples/locations")]
	locations: PathBuf,
}

#[derive(Subcommand)]
enum Cmd {
	/// Where the people are: demand under the competitors, as a map served from here. The page picks
	/// a trade and a city out of the two directories, a field per axis; naming both as files opens
	/// the one tab, already built. The trade field also offers none, which serves the city off the
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
	eyre::ensure!(!(refresh && cli.block_spend), "`--refresh` buys every answer again, and `--block-spend` buys none");
	let block_spend = cli.block_spend;
	let work = Work::at(v_utils::xdg_cache_dir!(""))
		.policy((&cfg.age).into(), (&cfg.sweep).into())
		.refresh(refresh)
		.block_spend(block_spend);
	match cli.cmd {
		Cmd::Config { .. } => unreachable!("handled above, and `handle_settings_command` exits"),
		Cmd::Schema => {
			println!("{}", serde_json::to_string_pretty(&schemars::schema_for!(gmaps_optimal_placement::Study))?);
			Ok(())
		}
		Cmd::Serve { at, port, open, refresh: _ } => {
			// the page picks out of the two directories, so `fzf` is not one of two pickers to learn. A
			// pair named as files is built here instead: its statistics print, and a study that will not
			// build fails before the listener binds
			let (trades, locations) = (gmaps_optimal_placement::files(&at.trades)?, gmaps_optimal_placement::files(&at.locations)?);
			let prebuilt = match (trades.as_slice(), locations.as_slice()) {
				([trade], [location]) => {
					let payload = gmaps_optimal_placement::load(Some(trade), location)?.build(&work)?;
					report(gmaps_optimal_placement::stats(&payload));
					vec![(
						Some(gmaps_optimal_placement::stem(trade).to_owned()),
						gmaps_optimal_placement::stem(location).to_owned(),
						serde_json::to_string(&payload)?,
					)]
				}
				_ => Vec::new(),
			};
			// the work dir rather than `work` itself: `Work` holds a `Cell`, and the server calls this
			// from whichever blocking thread a tab's first request landed on. `--refresh` does not come
			// along: it belongs to the run that asked for it, and a tab switch is not one — the page's own
			// reload is. `--block-spend` does, because it is about the server and not about a run
			let (dir, age, tiling) = (work.path().to_owned(), (&cfg.age).into(), (&cfg.sweep).into());
			let build = std::sync::Arc::new({
				let dir = dir.clone();
				move |trade: Option<&std::path::Path>, location: &std::path::Path, refresh: bool| {
					eyre::ensure!(!(refresh && block_spend), "`--block-spend` buys nothing, so it re-asks nothing either");
					let payload = gmaps_optimal_placement::load(trade, location)?.build(&Work::at(dir.clone()).policy(age, tiling).refresh(refresh).block_spend(block_spend))?;
					Ok(serde_json::to_string(&payload)?)
				}
			});
			let find = std::sync::Arc::new(move |location: &std::path::Path, q: &str| {
				let frame = gmaps_optimal_placement::load(None, location)?.area.bbox;
				gmaps_optimal_placement_sources::poi::find(&Work::at(dir.clone()).policy(age, tiling).block_spend(block_spend), frame, q)
			});
			gmaps_optimal_placement_web::serve::serve(trades, locations, prebuilt, build, find, SocketAddr::from(([127, 0, 0, 1], port)), open)
		}
		Cmd::Probe { at, dry_run, refresh: _ } => {
			let (trade, location) = (gmaps_optimal_placement::pick(&at.trades)?, gmaps_optimal_placement::pick(&at.locations)?);
			gmaps_optimal_placement::load(Some(&trade), &location)?.probe(&work, dry_run)?;
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
			let (trade, location) = (gmaps_optimal_placement::pick(&at.trades)?, gmaps_optimal_placement::pick(&at.locations)?);
			let payload = gmaps_optimal_placement::load(Some(&trade), &location)?.searches(&work)?;
			report(gmaps_optimal_placement::search_stats(&payload));
			let html = gmaps_optimal_placement::render::render_searches(&payload)?;
			write(out.unwrap_or_else(|| work.path().join("out").join(format!("{}-searches.html", payload.name))), &html)
		}
	}
}

/// Every pairing of the two axes that has been harvested, and what it collected. No picker: how
/// Google ranks is one mechanism, so every harvested pairing contributes. The studies come back
/// alongside, filtered to the ones that said something.
fn harvest(at: &Pair, work: &Work) -> Result<(Vec<Study>, Vec<Observed>)> {
	let (trades, locations) = (gmaps_optimal_placement::files(&at.trades)?, gmaps_optimal_placement::files(&at.locations)?);
	let studies = trades
		.iter()
		.flat_map(|t| locations.iter().map(move |l| gmaps_optimal_placement::load(Some(t), l)))
		.collect::<Result<Vec<_>>>()?;
	let (mut kept, mut observed) = (Vec::new(), Vec::new());
	for s in studies {
		if let Some(o) = s.observations(work)? {
			kept.push(s);
			observed.push(o);
		}
	}
	eyre::ensure!(!observed.is_empty(), "nothing under {} × {} has ever been harvested", at.trades.display(), at.locations.display());
	Ok((kept, observed))
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
