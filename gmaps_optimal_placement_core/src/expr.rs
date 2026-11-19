//! Arithmetic over grid columns, written in the study's TOML.
//!
//! The function surface is ours, not whatever the expression crate ships: every name a study may
//! write is registered here, and the built-ins are switched off so the config language cannot
//! drift under a dependency bump.
use evalexpr::{Context, ContextWithMutableFunctions, ContextWithMutableVariables, DefaultNumericTypes, EvalexprError, Function, HashMapContext, Node, Value};
use eyre::{Result, bail, eyre};
use indexmap::IndexMap;

type Tree = Node<DefaultNumericTypes>;
type Ctx = HashMapContext<DefaultNumericTypes>;
type Registered = (&'static str, usize, fn(&[f64]) -> f64);

#[derive(Clone, Debug)]
pub struct Expr {
	src: String,
	tree: Tree,
	vars: Vec<String>,
}

impl Expr {
	pub fn parse(src: &str) -> Result<Self> {
		let tree = evalexpr::build_operator_tree::<DefaultNumericTypes>(src).map_err(|e| eyre!("expression {src:?}: {e}"))?;
		let mut vars: Vec<String> = tree.iter_variable_identifiers().map(str::to_owned).collect();
		vars.sort();
		vars.dedup();
		Ok(Self { src: src.to_owned(), tree, vars })
	}

	/// Evaluate once per row. Fails on the first row that does not produce a number, so a study
	/// never gets a column of silent zeroes.
	pub fn eval_column(&self, columns: &IndexMap<String, Vec<f64>>, rows: usize) -> Result<Vec<f64>> {
		let mut ctx = context()?;
		let mut bound = Vec::with_capacity(self.vars.len());
		for name in &self.vars {
			let col = columns
				.get(name)
				.ok_or_else(|| eyre!("expression {:?} reads unknown column {name:?}; source has: {}", self.src, list(columns)))?;
			if col.len() != rows {
				bail!("column {name:?} has {} values, grid has {rows} cells", col.len());
			}
			bound.push((name.as_str(), col));
		}
		let mut out = Vec::with_capacity(rows);
		for i in 0..rows {
			for (name, col) in &bound {
				ctx.set_value((*name).to_owned(), Value::Float(col[i])).map_err(|e| eyre!("binding {name}: {e}"))?;
			}
			let v = self.tree.eval_with_context(&ctx).map_err(|e| eyre!("expression {:?} at cell {i}: {e}", self.src))?;
			let v = v.as_number().map_err(|e| eyre!("expression {:?} at cell {i} is not a number: {e}", self.src))?;
			if !v.is_finite() {
				bail!("expression {:?} is {v} at cell {i}", self.src);
			}
			out.push(v);
		}
		Ok(out)
	}
}

fn list<V>(m: &IndexMap<String, V>) -> String {
	m.keys().cloned().collect::<Vec<_>>().join(", ")
}

fn nums(v: &Value<DefaultNumericTypes>, want: usize) -> Result<Vec<f64>, EvalexprError<DefaultNumericTypes>> {
	let args = if want == 1 { vec![v.clone()] } else { v.as_fixed_len_tuple(want)?.to_vec() };
	args.iter().map(Value::as_number).collect()
}

fn context() -> Result<Ctx> {
	let mut ctx = Ctx::new();
	ctx.set_builtin_functions_disabled(true).map_err(|e| eyre!("disabling built-in functions: {e}"))?;
	// name, arity, body
	let fns: [Registered; 5] = [
		("max", 2, |a| a[0].max(a[1])),
		("min", 2, |a| a[0].min(a[1])),
		("clamp", 3, |a| a[0].clamp(a[1], a[2])),
		("sqrt", 1, |a| a[0].sqrt()),
		("pow", 2, |a| a[0].powf(a[1])),
	];
	for (name, arity, f) in fns {
		ctx.set_function(name.to_owned(), Function::new(move |v| Ok(Value::Float(f(&nums(v, arity)?)))))
			.map_err(|e| eyre!("registering {name}: {e}"))?;
	}
	Ok(ctx)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cols(pairs: &[(&str, &[f64])]) -> IndexMap<String, Vec<f64>> {
		pairs.iter().map(|(k, v)| ((*k).to_owned(), v.to_vec())).collect()
	}

	#[test]
	fn the_clermont_demand_expression() {
		let c = cols(&[("men_mais", &[10., 0.]), ("men_coll", &[0., 20.]), ("nv", &[22000., 11000.])]);
		let e = Expr::parse("(men_mais * 1.55 + men_coll * 0.85) * (max(nv, 4000) / 22000) ^ 1.6").unwrap();
		let v = e.eval_column(&c, 2).unwrap();
		assert!((v[0] - 15.5).abs() < 1e-9, "{v:?}");
		assert!((v[1] - 17. * 0.5f64.powf(1.6)).abs() < 1e-9, "{v:?}");
	}

	#[test]
	fn unknown_column_is_an_error_naming_what_exists() {
		let e = Expr::parse("popultaion * 2").unwrap();
		let err = e.eval_column(&cols(&[("population", &[1.])]), 1).unwrap_err().to_string();
		assert!(err.contains("popultaion"), "{err}");
		assert!(err.contains("population"), "{err}");
	}

	#[test]
	fn builtins_we_did_not_register_are_not_reachable() {
		assert!(Expr::parse("math::ln(2)").unwrap().eval_column(&IndexMap::new(), 1).is_err());
		assert!(Expr::parse("len(\"ab\")").unwrap().eval_column(&IndexMap::new(), 1).is_err());
	}
}
