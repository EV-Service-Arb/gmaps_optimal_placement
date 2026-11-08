//! Everything the sliders move: competitor pressure, underserved demand, the capture score and the
//! top-N sweep, plus the colouring and the report rows they feed.
//!
//! No I/O and no `google.maps` — the browser recomputes here, and `cargo t` checks the same code
//! against a fixture.
use std::borrow::Cow;

use eyre::{Result, ensure};
use serde::Serialize;

use crate::{
	payload::{Candidate, Payload, PoiOut, Scale, Trade},
	rank::{self, Biz, Feats, Rank},
};

const R: f64 = 6378137.;
/// Beyond 3λ the exponential contributes under 5 % of one competitor — not worth the distance.
const CUTOFF: f64 = 3.;

/// Inferno, from its first purple on. Warm and sequential: blue belongs to the pressure field, which
/// is painted over the heat and has to stay readable through it.
const RAMP: [[f64; 3]; 8] = [
	[40., 11., 84.],
	[87., 16., 110.],
	[135., 33., 107.],
	[181., 54., 89.],
	[221., 81., 58.],
	[246., 125., 21.],
	[250., 186., 31.],
	[245., 245., 145.],
];
/// The field's iso-lines, as quantiles of the pressure among the cells in view.
pub const LEVELS: [f64; 3] = [0.5, 0.75, 0.9];
/// Web-Mercator world coords, 256 px at zoom 0. Precomputed so a redraw is arithmetic instead of
/// 40k projection calls per frame.
pub fn world_x(lng: f64) -> f64 {
	(lng + 180.) / 360. * 256.
}

pub fn world_y(lat: f64) -> f64 {
	let s = (lat.to_radians()).sin();
	128. - 256. / (4. * std::f64::consts::PI) * ((1. + s) / (1. - s)).ln()
}

/// Live tier state — the checkbox and the weight slider.
#[derive(Clone, Debug)]
pub struct TierState {
	pub name: String,
	pub weight: f64,
	pub show: bool,
}
/// Which array a layer paints. A study layer was evaluated at build time and needs nothing; the
/// rest are functions of the sliders, and of a trade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerRef {
	Study(usize),
	Trade(TradeLayer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TradeLayer {
	Unmet,
	Demand,
	/// One competitor's share of the cell under the fitted model.
	Share,
	/// The same competitor's reach as the probe observed it.
	Seen,
}
pub struct LayerSpec {
	pub name: String,
	pub note: String,
	/// Magnitude rather than rank.
	pub linear: bool,
	pub source: LayerRef,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Row {
	pub label: String,
	/// Second line under the label: whatever the row measured against.
	pub sub: Option<String>,
	/// Empty makes the row a section heading.
	pub value: String,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
	pub title: String,
	pub rows: Vec<Row>,
	pub note: String,
}
/// A cell the top-N sweep picked.
#[derive(Debug, Serialize)]
pub struct Ranked {
	pub cell: usize,
	pub lat: f64,
	pub lng: f64,
	pub score: f64,
}
/// The study's geometry, projected once. Everything below reads it and the live control values.
pub struct Model {
	pub payload: Payload,
	/// `n * 4` Web-Mercator corners, in the canvas loop's own order.
	pub ring_x: Vec<f64>,
	pub ring_y: Vec<f64>,
	pub c_lat: Vec<f64>,
	pub c_lng: Vec<f64>,
	/// Cell centroids in metres from the grid's own centre. Flat-earth over one agglomeration.
	mx: Vec<f64>,
	my: Vec<f64>,
	lat0: f64,
	lng0: f64,
	m_per_lng: f64,
	m_per_lat: f64,
	/// Every ring corner four cells meet at, as those cells: the squares the iso-lines are traced
	/// over, centroid to centroid. Lower-left, lower-right, upper-right, upper-left.
	quads: Vec<[usize; 4]>,
	/// Present exactly when `payload.trade` is.
	trade: Option<TradeState>,
}

/// What projecting and rescoring the trade half bought. Paired with `Payload::trade` and never
/// without it.
struct TradeState {
	comps: Vec<Comp>,
	/// Probe nodes in the cells' frame, in `Trade::nodes` order. Empty for an unprobed study.
	nx: Vec<f64>,
	ny: Vec<f64>,
	/// The same fitted model `build` weighted the competitors with, so the what-if is scored on the
	/// scale the map is already painted in.
	rank: Rank,
	terms: Vec<(String, f64)>,
	/// The divisor `build` normalised the first tier by. A newcomer is measured against direct
	/// competition, which is what the study's first tier is.
	scale: f64,
}
impl Model {
	pub fn try_new(payload: Payload) -> Result<Self> {
		let n = payload.place.len();
		ensure!(n > 0, "payload carries no cells");
		ensure!(payload.ring.len() == n * 8, "payload carries {} ring numbers for {n} cells", payload.ring.len());
		ensure!(payload.imputed.len() == n, "payload arrays disagree on the cell count");
		for l in &payload.layers {
			ensure!(l.values.len() == n, "layer {:?} carries {} values for {n} cells", l.name, l.values.len());
		}

		let (mut ring_x, mut ring_y) = (vec![0.; n * 4], vec![0.; n * 4]);
		let (mut c_lat, mut c_lng) = (vec![0.; n], vec![0.; n]);
		let (mut lat0, mut lng0) = (0., 0.);
		for i in 0..n {
			let (mut sx, mut sy) = (0., 0.);
			for k in 0..4 {
				let (lng, lat) = (payload.ring[i * 8 + k * 2], payload.ring[i * 8 + k * 2 + 1]);
				ring_x[i * 4 + k] = world_x(lng);
				ring_y[i * 4 + k] = world_y(lat);
				sx += lng;
				sy += lat;
			}
			c_lng[i] = sx / 4.;
			c_lat[i] = sy / 4.;
			lat0 += c_lat[i];
			lng0 += c_lng[i];
		}
		lat0 /= n as f64;
		lng0 /= n as f64;
		let m_per_lng = std::f64::consts::PI / 180. * R * lat0.to_radians().cos();
		let m_per_lat = std::f64::consts::PI / 180. * R;
		let local = |lat: f64, lng: f64| ((lng - lng0) * m_per_lng, (lat - lat0) * m_per_lat);

		let (mut mx, mut my) = (vec![0.; n], vec![0.; n]);
		for i in 0..n {
			(mx[i], my[i]) = local(c_lat[i], c_lng[i]);
		}
		// adjacent cells reproject one 3035 corner to the same rounded floats, so a corner is its bits
		let mut corners: std::collections::HashMap<(u64, u64), [Option<usize>; 4]> = std::collections::HashMap::new();
		for i in 0..n {
			for (k, slot) in [2, 3, 0, 1].into_iter().enumerate() {
				let at = (payload.ring[i * 8 + k * 2].to_bits(), payload.ring[i * 8 + k * 2 + 1].to_bits());
				let seat = &mut corners.entry(at).or_default()[slot];
				if let Some(j) = seat {
					eyre::bail!("cells {j} and {i} both claim one corner of the grid");
				}
				*seat = Some(i);
			}
		}
		let quads = corners.into_values().filter_map(|q| Some([q[0]?, q[1]?, q[2]?, q[3]?])).collect();
		let trade = payload
			.trade
			.as_ref()
			.map(|t| {
				ensure!(t.demand.len() == n, "payload carries {} demand values for {n} cells", t.demand.len());
				let (mut nx, mut ny) = (Vec::new(), Vec::new());
				for at in &t.nodes {
					let (x, y) = local(at[0], at[1]);
					nx.push(x);
					ny.push(y);
				}
				for p in &t.pois {
					ensure!(
						p.seen.len() == t.nodes.len(),
						"competitor {:?} carries {} node observations against {} nodes",
						p.poi.name,
						p.seen.len(),
						t.nodes.len()
					);
				}
				let comps = t
					.pois
					.iter()
					.map(|p| {
						let tier = t
							.tiers
							.iter()
							.position(|x| x.name == p.poi.tier)
							.ok_or_else(|| eyre::eyre!("competitor {:?} is in tier {:?}, which the payload does not declare", p.poi.name, p.poi.tier))?;
						let (mx, my) = local(p.poi.lat, p.poi.lng);
						Ok(Comp { mx, my, w: p.w, tier })
					})
					.collect::<Result<Vec<_>>>()?;

				let rated: Vec<f64> = t.pois.iter().filter_map(|p| p.poi.rating).collect();
				ensure!(!rated.is_empty(), "no competitor carries a rating, so there is nothing to shrink towards");
				let rank = Rank::try_new(Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, &payload.place)?, rank::COEF)?;
				let terms: Vec<(String, f64)> = t.terms.iter().map(|x| (x.text.clone(), x.weight)).collect();

				// rescoring the first tier here recovers the divisor `build` used, and proves the coefficients
				// that painted this payload are the ones linked in: otherwise the map would quietly show
				// weights from a model nobody is running any more
				let first = &t.tiers.first().ok_or_else(|| eyre::eyre!("payload declares no tier"))?.name;
				let mut scored: Vec<(f64, f64)> = t.pois.iter().filter(|p| &p.poi.tier == first).map(|p| (rank.strength(&Biz::from(&p.poi), &terms), p.w)).collect();
				ensure!(!scored.is_empty(), "no competitor is in tier {first:?}, which is the scale everything else is read against");
				scored.sort_by(|a, b| a.0.total_cmp(&b.0));
				let scale = scored[scored.len() / 2].0;
				ensure!(scale > 0., "tier {first:?} scores a median strength of {scale}");
				for (raw, w) in &scored {
					ensure!(
						(raw / scale - w).abs() < 1e-3,
						"tier {first:?} carries w={w} where rank::COEF now scores {:.4} — the payload was built by a different model",
						raw / scale
					);
				}
				Ok(TradeState { comps, nx, ny, rank, terms, scale })
			})
			.transpose()?;

		Ok(Self {
			payload,
			ring_x,
			ring_y,
			c_lat,
			c_lng,
			mx,
			my,
			lat0,
			lng0,
			m_per_lng,
			m_per_lat,
			quads,
			trade,
		})
	}

	/// Everything a slider moves, or nothing. A map opened without a trade has no demand surface and
	/// no competitors, so the quantities that read either are not reachable from here at all.
	pub fn traded(&self) -> Option<Traded<'_>> {
		Some(Traded {
			model: self,
			trade: self.payload.trade.as_ref()?,
			state: self.trade.as_ref()?,
		})
	}

	pub fn n(&self) -> usize {
		self.payload.place.len()
	}

	/// The tier controls at their opening positions. Empty without a trade: there is nothing on the
	/// ground to weigh.
	pub fn tier_states(&self) -> Vec<TierState> {
		self.payload
			.trade
			.iter()
			.flat_map(|t| &t.tiers)
			.map(|t| TierState {
				name: t.name.clone(),
				weight: t.weight,
				show: true,
			})
			.collect()
	}

	pub fn to_local(&self, lat: f64, lng: f64) -> (f64, f64) {
		((lng - self.lng0) * self.m_per_lng, (lat - self.lat0) * self.m_per_lat)
	}

	/// The ones that move under the sliders, then whatever the study asked for. Without a trade only
	/// the second half exists — every layer above it is a function of demand, competitors, or both.
	pub fn layers(&self) -> Vec<LayerSpec> {
		let mut out = Vec::new();
		if let Some(t) = &self.payload.trade {
			out.extend([
				LayerSpec {
					name: "Underserved demand  ★".to_owned(),
					note: "Demand in the cell, divided down by how much competition already reaches it. This is the layer to shop for locations on.".to_owned(),
					linear: false,
					source: LayerRef::Trade(TradeLayer::Unmet),
				},
				LayerSpec {
					name: "Demand".to_owned(),
					note: t.demand_note.clone(),
					linear: false,
					source: LayerRef::Trade(TradeLayer::Demand),
				},
				LayerSpec {
					name: "Coverage · modelled".to_owned(),
					note: "The highlighted competitor's share of the cell: its strength against everything else reaching there. One global distance coefficient, so it is a circle — which is the control the observed layer is read against. Click a competitor to move the highlight.".to_owned(),
					linear: true,
					source: LayerRef::Trade(TradeLayer::Share),
				},
			]);
			if !t.nodes.is_empty() {
				out.push(LayerSpec {
					name: "Coverage · observed  ★".to_owned(),
					note: format!(
						"What the probe saw of the highlighted competitor at each of {} nodes, interpolated between them. Pure data: rank on the page, term weights applied, nothing modelled.",
						t.nodes.len()
					),
					linear: true,
					source: LayerRef::Trade(TradeLayer::Seen),
				});
			}
		}
		out.extend(self.payload.layers.iter().enumerate().map(|(i, l)| LayerSpec {
			name: l.name.clone(),
			note: l.note.clone(),
			linear: matches!(l.scale, Scale::Linear),
			source: LayerRef::Study(i),
		}));
		out
	}

	/// What the study's own expressions evaluated to at build time. The rest of the layers are
	/// [`Traded::values`], because the rest of the layers need a trade.
	pub fn layer(&self, i: usize) -> &[f64] {
		&self.payload.layers[i].values
	}

	/// Iso-lines of `values` at each of `levels`, traced by marching squares between cell centroids:
	/// `[x0, y0, x1, y1, level]` per segment, in the Web-Mercator world coords `ring_x`/`ring_y` are in.
	/// A hole in the grid ends a line rather than closing it.
	pub fn contours(&self, values: &[f64], levels: &[f64]) -> Vec<f64> {
		assert_eq!(values.len(), self.n(), "one value per cell");
		let at = |i: usize| -> [f64; 2] {
			let (x, y) = (&self.ring_x[i * 4..i * 4 + 4], &self.ring_y[i * 4..i * 4 + 4]);
			[x.iter().sum::<f64>() / 4., y.iter().sum::<f64>() / 4.]
		};
		let mut out = Vec::new();
		for q in &self.quads {
			let v = q.map(|i| values[i]);
			let p = q.map(at);
			for (li, &level) in levels.iter().enumerate() {
				let case = (0..4).fold(0, |c, k| c | (usize::from(v[k] >= level) << k));
				let cut = |e: usize| -> [f64; 2] {
					let (a, b) = (e, (e + 1) % 4);
					let t = (level - v[a]) / (v[b] - v[a]);
					[p[a][0] + t * (p[b][0] - p[a][0]), p[a][1] + t * (p[b][1] - p[a][1])]
				};
				let centre = v.iter().sum::<f64>() / 4. >= level;
				let edges: &[[usize; 2]] = match case {
					0 | 15 => &[],
					1 | 14 => &[[3, 0]],
					2 | 13 => &[[0, 1]],
					3 | 12 => &[[3, 1]],
					4 | 11 => &[[1, 2]],
					6 | 9 => &[[0, 2]],
					7 | 8 => &[[3, 2]],
					5 if centre => &[[0, 1], [2, 3]],
					5 => &[[3, 0], [1, 2]],
					10 if centre => &[[3, 0], [1, 2]],
					10 => &[[0, 1], [2, 3]],
					_ => unreachable!("four corners make sixteen cases"),
				};
				for [a, b] in edges {
					let (a, b) = (cut(*a), cut(*b));
					out.extend([a[0], a[1], b[0], b[1], li as f64]);
				}
			}
		}
		out
	}

	pub fn nearest_cell(&self, mx: f64, my: f64) -> usize {
		(0..self.n())
			.min_by(|&a, &b| (self.mx[a] - mx).hypot(self.my[a] - my).total_cmp(&(self.mx[b] - mx).hypot(self.my[b] - my)))
			.expect("the model rejects an empty grid")
	}

	/// The cell under the cursor, on the same ~120 m tolerance the map used. `None` off the grid.
	pub fn cell_at(&self, lat: f64, lng: f64, shown: &[u8]) -> Option<usize> {
		let d_lat = 0.0011;
		let d_lng = 0.0011 / lat.to_radians().cos();
		(0..self.n()).find(|&i| shown[i] != 0 && (self.c_lat[i] - lat).abs() < d_lat && (self.c_lng[i] - lng).abs() < d_lng)
	}

	pub fn tooltip(&self, i: usize, press: &[f64], unmet: &[f64]) -> String {
		let mut s = self.payload.place[i].clone();
		for l in &self.payload.layers {
			s.push_str(&format!("\n{} {}", l.name, fmt(l.values[i])));
		}
		if let Some(t) = &self.payload.trade {
			s.push_str(&format!("\ndemand {} · pressure {:.2} · unmet {}", fmt(t.demand[i]), press[i], fmt(unmet[i])));
		}
		if self.payload.imputed[i] == 1 {
			s.push_str("\n(imputed cell)");
		}
		s
	}

	/// What the statistical grid publishes about the cell a point lands in. The whole of what a map
	/// with no trade can say about a site, and the head of what one with a trade says.
	pub fn cell_report(&self, at: [f64; 2], label: Option<&str>) -> Report {
		let [lat, lng] = at;
		let (mx, my) = self.to_local(lat, lng);
		let cell = self.nearest_cell(mx, my);
		let mut rows: Vec<Row> = self.payload.layers.iter().map(|l| row(&l.name, fmt(l.values[cell]))).collect();
		rows.push(row("Coordinates", format!("{lat:.5}, {lng:.5}")));
		Report {
			title: self.titled(label, cell),
			rows,
			note: "This cell as the statistical grid publishes it. Pick a trade to get demand, the competitors already on the ground, and a capture score.".to_owned(),
		}
	}

	fn titled(&self, label: Option<&str>, cell: usize) -> String {
		match label {
			Some(l) => format!("{l} — {}", self.payload.place[cell]),
			None => self.payload.place[cell].clone(),
		}
	}
}

/// A model that has a trade under it: a demand surface, an inventory, and the fitted weights that
/// scored it. Reachable only through [`Model::traded`], so every quantity below is asked of a map
/// that has one.
#[derive(Clone, Copy)]
pub struct Traded<'a> {
	pub model: &'a Model,
	pub trade: &'a Trade,
	state: &'a TradeState,
}

impl<'a> Traded<'a> {
	/// Σ over competitors of weight × exp(−distance/λ).
	pub fn pressure(self, lambda: f64, tiers: &[TierState]) -> Vec<f64> {
		let cut2 = (CUTOFF * lambda) * (CUTOFF * lambda);
		let mut press = vec![0.; self.model.n()];
		for c in &self.state.comps {
			let t = &tiers[c.tier];
			if !t.show {
				continue;
			}
			let base = c.w * t.weight;
			if base <= 0. {
				continue;
			}
			for (i, p) in press.iter_mut().enumerate() {
				let (dx, dy) = (self.model.mx[i] - c.mx, self.model.my[i] - c.my);
				let d2 = dx * dx + dy * dy;
				if d2 > cut2 {
					continue;
				}
				*p += base * (-d2.sqrt() / lambda).exp();
			}
		}
		press
	}

	pub fn unmet(self, press: &[f64]) -> Vec<f64> {
		self.trade.demand.iter().zip(press).map(|(d, p)| d / (1. + p)).collect()
	}

	/// One competitor's coverage cloud, cell by cell. `Share` is the Plackett–Luce share it takes of
	/// the cell: its distance-decayed strength over everything else reaching there, which sums to one
	/// across competitors and so reads as market share rather than as an unlabelled glow. `Seen` is
	/// what the probe found, interpolated between the nodes it stood at — inverse square, so there is
	/// no bandwidth to justify.
	///
	/// The denominator runs without `pressure`'s cutoff: a share is a ratio, and truncating only the
	/// bottom of it moves the answer where `pressure` only loses a rounding error.
	fn cloud(self, source: TradeLayer, of: usize, lambda: f64, tiers: &[TierState]) -> Vec<f64> {
		let n = self.model.n();
		let (t, tm) = (self.trade, self.state);
		let Some(comp) = tm.comps.get(of) else { return vec![0.; n] };
		match source {
			TradeLayer::Seen => {
				let seen = &t.pois[of].seen;
				(0..n)
					.map(|i| {
						// the three nearest nodes and no others: over all of them the far field would
						// converge on the study-wide average, and a competitor nobody saw across the river
						// would read as a wash rather than as the zero it is. Three is what defines a plane.
						let mut d: Vec<(f64, f64)> = seen
							.iter()
							.enumerate()
							// a cell standing on a node takes that node's observation and nothing else
							.map(|(k, s)| ((self.model.mx[i] - tm.nx[k]).hypot(self.model.my[i] - tm.ny[k]).powi(2).max(1.), *s))
							.collect();
						d.sort_by(|a, b| a.0.total_cmp(&b.0));
						let (mut num, mut den) = (0., 0.);
						for (d2, s) in d.iter().take(3) {
							num += s / d2;
							den += 1. / d2;
						}
						match den > 0. {
							true => num / den,
							false => 0.,
						}
					})
					.collect()
			}
			_ => {
				let pull = |c: &Comp, i: usize| c.w * tiers[c.tier].weight * (-(self.model.mx[i] - c.mx).hypot(self.model.my[i] - c.my) / lambda).exp();
				let mut den = vec![0.; n];
				for c in tm.comps.iter().filter(|c| tiers[c.tier].show && c.w * tiers[c.tier].weight > 0.) {
					for (i, d) in den.iter_mut().enumerate() {
						*d += pull(c, i);
					}
				}
				match tiers[comp.tier].show {
					true => (0..n).map(|i| if den[i] > 0. { pull(comp, i) / den[i] } else { 0. }).collect(),
					false => vec![0.; n],
				}
			}
		}
	}

	/// Huff-style: a candidate captures each cell's demand in proportion to its own distance-decayed
	/// pull against all competition already reaching that cell.
	pub fn capture_at(self, mx: f64, my: f64, lambda: f64, press: &[f64]) -> f64 {
		let demand = &self.trade.demand;
		let cut2 = (CUTOFF * lambda) * (CUTOFF * lambda);
		let mut cap = 0.;
		for (i, p) in press.iter().enumerate() {
			let (dx, dy) = (self.model.mx[i] - mx, self.model.my[i] - my);
			let d2 = dx * dx + dy * dy;
			if d2 > cut2 {
				continue;
			}
			let pull = (-d2.sqrt() / lambda).exp();
			cap += demand[i] * pull / (pull + p);
		}
		cap
	}

	/// Demand within 1 km and within 3 km.
	pub fn demand_within(self, mx: f64, my: f64) -> (f64, f64) {
		let demand = &self.trade.demand;
		let (mut d1, mut d3) = (0., 0.);
		for i in 0..self.model.n() {
			let (dx, dy) = (self.model.mx[i] - mx, self.model.my[i] - my);
			let d2 = dx * dx + dy * dy;
			if d2 > 9e6 {
				continue;
			}
			d3 += demand[i];
			if d2 < 1e6 {
				d1 += demand[i];
			}
		}
		(d1, d3)
	}

	/// Closest competitor and its distance in metres, over the whole inventory or one tier. Tier
	/// visibility does not enter: what is on the ground is on the ground.
	pub fn nearest(self, mx: f64, my: f64, tier: Option<&str>) -> Option<(&'a PoiOut, f64)> {
		let (t, tm) = (self.trade, self.state);
		t.pois
			.iter()
			.zip(&tm.comps)
			.filter(|(p, _)| tier.is_none_or(|t| p.poi.tier == t))
			.map(|(p, c)| (p, (c.mx - mx).hypot(c.my - my)))
			.min_by(|a, b| a.1.total_cmp(&b.1))
	}

	pub fn within(self, mx: f64, my: f64, r: f64, tier: Option<&str>) -> usize {
		let (t, tm) = (self.trade, self.state);
		t.pois
			.iter()
			.zip(&tm.comps)
			.filter(|(p, c)| tier.is_none_or(|t| p.poi.tier == t) && (c.mx - mx).hypot(c.my - my) < r)
			.count()
	}

	/// Greedy top-N sweep over cell centroids, one candidate per 400 m and picks kept 1.5 km apart.
	/// Returns the picks and how many cells were in the running.
	pub fn rank_sites(self, lambda: f64, press: &[f64], take: usize) -> (Vec<Ranked>, usize) {
		let demand = &self.trade.demand;
		// one candidate per 400 m: 200 m spacing buys nothing and quadruples the cost
		let mut seen = std::collections::HashSet::new();
		let mut cand = Vec::new();
		for i in 0..self.model.n() {
			if demand[i] <= 0. {
				continue;
			}
			let key = ((self.model.mx[i] / 400. + 0.5).floor() as i64, (self.model.my[i] / 400. + 0.5).floor() as i64);
			if seen.insert(key) {
				cand.push(i);
			}
		}
		let mut scored: Vec<(usize, f64)> = cand.iter().map(|&i| (i, self.capture_at(self.model.mx[i], self.model.my[i], lambda, press))).collect();
		scored.sort_by(|a, b| b.1.total_cmp(&a.1));

		let mut picked: Vec<Ranked> = Vec::new();
		for (i, score) in scored {
			if picked
				.iter()
				.all(|p| (self.model.mx[p.cell] - self.model.mx[i]).hypot(self.model.my[p.cell] - self.model.my[i]) > 1500.)
			{
				picked.push(Ranked {
					cell: i,
					lat: self.model.c_lat[i],
					lng: self.model.c_lng[i],
					score,
				});
			}
			if picked.len() == take {
				break;
			}
		}
		(picked, cand.len())
	}

	/// Open here, under this name, with nothing on the board yet: what would Google's own ordering
	/// make of it. Scored on the same per-tier scale `w` is, so `1.00` is the median rival.
	pub fn opening_weight(self, name: &str, lat: f64, lng: f64) -> f64 {
		let tm = self.state;
		tm.rank.strength(
			&Biz {
				name,
				n_rev: 0.,
				rating: None,
				lat,
				lng,
			},
			&tm.terms,
		) / tm.scale
	}

	fn what_if(self, name: &str, mx: f64, my: f64, lat: f64, lng: f64) -> Vec<Row> {
		let w = self.opening_weight(name, lat, lng);
		let (t, tm) = (self.trade, self.state);
		let near: Vec<f64> = t.pois.iter().zip(&tm.comps).filter(|(_, c)| (c.mx - mx).hypot(c.my - my) < 2000.).map(|(p, _)| p.w).collect();
		vec![
			Row {
				label: format!("Open as {name:?}, no reviews"),
				sub: Some("reviews are associated with rank, not a lever on it".to_owned()),
				value: format!("{w:.2} of a median rival"),
			},
			row("Outranks, of the ≤2 km field", format!("{} of {}", near.iter().filter(|&&x| x < w).count(), near.len())),
		]
	}

	pub fn compare(self, candidates: &[Candidate], lambda: f64, press: &[f64]) -> Report {
		struct Line {
			name: String,
			cap: f64,
			d1: f64,
			d3: f64,
			near: Option<f64>,
			n2: usize,
			w: f64,
		}
		let lines: Vec<Line> = candidates
			.iter()
			.map(|c| {
				let (mx, my) = self.model.to_local(c.at[0], c.at[1]);
				let (d1, d3) = self.demand_within(mx, my);
				Line {
					name: c.name.clone(),
					cap: self.capture_at(mx, my, lambda, press),
					d1,
					d3,
					near: self.nearest(mx, my, None).map(|(_, d)| d),
					n2: self.within(mx, my, 2000., None),
					w: self.opening_weight(&c.name, c.at[0], c.at[1]),
				}
			})
			.collect();
		let best = lines.iter().map(|l| l.cap).fold(f64::NEG_INFINITY, f64::max);

		let mut rows = vec![heading("Capture score")];
		rows.extend(lines.iter().map(|l| row(&l.name, format!("{} · {:.0}%", fmt(l.cap), 100. * l.cap / best))));
		rows.push(heading("Demand ≤1 / ≤3 km"));
		rows.extend(lines.iter().map(|l| row(&l.name, format!("{} / {}", fmt(l.d1), fmt(l.d3)))));
		rows.push(heading("Nearest competitor · ≤2 km"));
		rows.extend(lines.iter().map(|l| {
			row(
				&l.name,
				match l.near {
					Some(d) => format!("{:.2} km · {}", d / 1000., l.n2),
					None => format!("– · {}", l.n2),
				},
			)
		}));
		rows.push(heading("Opening weight, under this name, no reviews"));
		rows.extend(lines.iter().map(|l| row(&l.name, format!("{:.2} of a median rival", l.w))));

		Report {
			title: format!("Candidates · λ={:.1} km", lambda / 1000.),
			rows,
			note: "Percentages are against the best candidate here, not against the best site in the area — use \"Rank top 10 sites\" for that. Opening weight is what Google's ordering makes of the name; reviews are associated with rank, not a lever on it.".to_owned(),
		}
	}

	/// `label` titles the card; `name` is the trading name to score the what-if under, which is a
	/// different string because the title carries the pin's letter and the model reads the words.
	pub fn site_report(self, at: [f64; 2], label: Option<&str>, name: Option<&str>, lambda: f64, press: &[f64], tiers: &[TierState]) -> Report {
		let [lat, lng] = at;
		let (mx, my) = self.model.to_local(lat, lng);
		let (d1, d3) = self.demand_within(mx, my);
		let any = self.nearest(mx, my, None);
		let km = |d: Option<f64>| d.map_or_else(|| "–".to_owned(), |d| format!("{:.2} km", d / 1000.));

		let mut rows = vec![
			row("Capture score", fmt(self.capture_at(mx, my, lambda, press))),
			row("Demand ≤1 / ≤3 km", format!("{} / {}", fmt(d1), fmt(d3))),
		];
		for t in tiers {
			let near = self.nearest(mx, my, Some(&t.name));
			rows.push(Row {
				label: format!("Nearest {}", t.name),
				sub: Some(near.map_or_else(|| "–".to_owned(), |(p, _)| p.poi.name.chars().take(28).collect())),
				value: km(near.map(|(_, d)| d)),
			});
			rows.push(row(
				format!("{} ≤2 / ≤5 km", t.name),
				format!("{} / {}", self.within(mx, my, 2000., Some(&t.name)), self.within(mx, my, 5000., Some(&t.name))),
			));
		}
		rows.push(row("Nearest competitor, any", km(any.map(|(_, d)| d))));
		rows.push(row("All competitors ≤2 km", self.within(mx, my, 2000., None).to_string()));
		rows.push(row("Coordinates", format!("{lat:.5}, {lng:.5}")));
		if let Some(name) = name {
			rows.extend(self.what_if(name, mx, my, lat, lng));
		}

		Report {
			title: self.model.titled(label, self.model.nearest_cell(mx, my)),
			rows,
			note: "Capture = Σ demand × pull/(pull + existing pressure). Only meaningful when comparing candidates against each other; the number has no unit.".to_owned(),
		}
	}

	/// `of` is the competitor the coverage layers are about, and is ignored by the rest.
	pub fn values(self, source: TradeLayer, of: usize, lambda: f64, tiers: &[TierState], unmet: &'a [f64]) -> Cow<'a, [f64]> {
		match source {
			TradeLayer::Unmet => Cow::Borrowed(unmet),
			TradeLayer::Demand => Cow::Borrowed(&self.trade.demand),
			TradeLayer::Share | TradeLayer::Seen => Cow::Owned(self.cloud(source, of, lambda, tiers)),
		}
	}
}

pub fn ramp(t: f64) -> [f64; 3] {
	let t = t.clamp(0., 1.) * (RAMP.len() - 1) as f64;
	let i = (t.floor() as usize).min(RAMP.len() - 2);
	let f = t - i as f64;
	let (a, b) = (RAMP[i], RAMP[i + 1]);
	[a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}
pub struct Colorised {
	/// `n * 3` RGB. Only the entries `shown` marks are meaningful.
	pub colors: Vec<u8>,
	pub shown: Vec<u8>,
	/// The legend's five labels, at 0/25/50/75/100 % — of the cells in view.
	pub ticks: [f64; 5],
	/// Shown cells in view.
	pub count: usize,
}
/// The ramp spans the shown cells in view, so a city zoomed into is coloured against itself; cells
/// outside it are coloured on the same ramp, clamped.
pub fn colorise(values: &[f64], imputed: &[u8], hide_imputed: bool, linear: bool, in_view: &[bool]) -> Colorised {
	let n = values.len();
	assert_eq!(in_view.len(), n, "one view flag per cell");
	let mut shown = vec![0u8; n];
	let mut idx = Vec::new();
	for i in 0..n {
		shown[i] = u8::from(!(hide_imputed && imputed[i] == 1) && values[i] > 0.);
		if shown[i] == 1 {
			idx.push(i);
		}
	}
	let mut sorted: Vec<f64> = idx.iter().filter(|&&i| in_view[i]).map(|&i| values[i]).collect();
	sorted.sort_by(f64::total_cmp);
	let count = sorted.len();
	let (lo, hi) = if count > 0 { (sorted[0], sorted[count - 1]) } else { (0., 1.) };

	let mut colors = vec![0u8; n * 3];
	for &i in &idx {
		let t = if linear {
			(values[i] - lo) / if hi - lo == 0. { 1. } else { hi - lo }
		} else {
			percentile(&sorted, values[i])
		};
		let c = ramp(t);
		colors[i * 3] = c[0] as u8;
		colors[i * 3 + 1] = c[1] as u8;
		colors[i * 3 + 2] = c[2] as u8;
	}
	let at = |f: f64| {
		if count == 0 {
			0.
		} else if linear {
			lo + f * (hi - lo)
		} else {
			sorted[(f * (count - 1) as f64).floor() as usize]
		}
	};
	Colorised {
		colors,
		shown,
		ticks: [at(0.), at(0.25), at(0.5), at(0.75), at(1.)],
		count,
	}
}

/// Rank within `sorted`, 0..=1; a value past either end is at that end.
fn percentile(sorted: &[f64], v: f64) -> f64 {
	(sorted.partition_point(|&s| s < v) as f64 / if sorted.len() > 1 { (sorted.len() - 1) as f64 } else { 1. }).min(1.)
}

/// The pressure field over the cells in view: an alpha per cell rising with its percentile there,
/// and the values at [`LEVELS`] its iso-lines are drawn at. `None` when no cell in view is under any
/// pressure.
pub struct Field {
	pub alpha: Vec<u8>,
	pub levels: [f64; 3],
}
pub fn field(values: &[f64], in_view: &[bool]) -> Option<Field> {
	assert_eq!(in_view.len(), values.len(), "one view flag per cell");
	let mut sorted: Vec<f64> = values.iter().zip(in_view).filter(|(v, seen)| **seen && **v > 0.).map(|(v, _)| *v).collect();
	if sorted.is_empty() {
		return None;
	}
	sorted.sort_by(f64::total_cmp);
	let alpha = values
		.iter()
		.map(|&v| match v > 0. {
			true => (haze(percentile(&sorted, v)) * 255.).round() as u8,
			false => 0,
		})
		.collect();
	Some(Field {
		alpha,
		levels: LEVELS.map(|q| sorted[(q * (sorted.len() - 1) as f64).floor() as usize]),
	})
}
/// The field's colour. Nothing in [`ramp`] comes near it.
pub const HAZE: [u8; 3] = [37, 99, 235];
/// The haze's alpha at percentile `p` of the pressure in view. Low enough that the yellow end of the
/// heat does not wash out under it: the iso-lines carry the reading, the haze only says where.
pub fn haze(p: f64) -> f64 {
	0.3 * p.powf(1.6)
}

/// A Google category, as Google files it — not as the study's `tier` rules read it. The tiers are
/// the study's argument about what competes; this is the evidence they were argued from, and the
/// two disagreeing is worth being able to see.
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
	/// Places `primaryType`, which the colour is keyed by.
	pub kind: String,
	/// The Business Profile category Google displays. A place it typed as nothing is still on the
	/// map and still says so.
	pub label: String,
	pub color: String,
	/// Per tier, in the payload's order, so a panel can count only what is on screen.
	pub counts: Vec<usize>,
}

/// Every category on the map, commonest first, each with the colour it is painted in. Golden-angle
/// hues, so neighbours in the list land far apart however many a city turns out to have — over a
/// wheel with blue cut out, which is the pressure field's.
pub fn categories(trade: &Trade) -> Vec<Category> {
	let mut out: Vec<Category> = Vec::new();
	for p in &trade.pois {
		let ti = trade.tiers.iter().position(|t| t.name == p.poi.tier).expect("the model rejected a payload whose tiers disagree");
		let i = match out.iter().position(|c| c.kind == p.poi.kind) {
			Some(i) => i,
			None => {
				out.push(Category {
					kind: p.poi.kind.clone(),
					label: match (p.poi.kind_label.as_str(), p.poi.kind.as_str()) {
						("", "") => "no category".to_owned(),
						("", k) => k.to_owned(),
						(l, _) => l.to_owned(),
					},
					color: String::new(),
					counts: vec![0; trade.tiers.len()],
				});
				out.len() - 1
			}
		};
		out[i].counts[ti] += 1;
	}
	out.sort_by_key(|c| std::cmp::Reverse(c.counts.iter().sum::<usize>()));
	for (i, c) in out.iter_mut().enumerate() {
		let h = (i as f64 * 137.508) % 360. * 290. / 360.;
		c.color = format!("hsl({:.0},72%,58%)", if h >= 190. { h + 70. } else { h });
	}
	out
}

/// Three significant-ish digits with an SI-ish suffix. Report and legend both read it, so a change
/// here moves every number on the page at once.
pub fn fmt(n: f64) -> String {
	if n >= 1e6 {
		format!("{:.2}M", n / 1e6)
	} else if n >= 1e4 {
		format!("{:.0}k", n / 1000.)
	} else if n >= 1000. {
		format!("{:.1}k", n / 1000.)
	} else if n >= 10. {
		format!("{n:.0}")
	} else {
		format!("{n:.2}")
	}
}
/// One competitor, reduced to what the model reads.
struct Comp {
	mx: f64,
	my: f64,
	/// Fitted strength, before the tier multiplier.
	w: f64,
	tier: usize,
}

fn row(label: impl Into<String>, value: impl Into<String>) -> Row {
	Row {
		label: label.into(),
		sub: None,
		value: value.into(),
	}
}

fn heading(label: impl Into<String>) -> Row {
	Row {
		label: label.into(),
		sub: None,
		value: String::new(),
	}
}
