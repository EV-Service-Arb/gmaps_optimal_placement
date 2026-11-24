//! The page's stylesheet: Tailwind over the kit's class inventory and this crate's views, with
//! `map.css` in a layer beneath the utilities. Only the server inlines it, so only `ssr` builds it.
//! The inventory and tokens are written out because tailwind can neither scan nor `@import` a crate
//! unpacked from crates.io.

use std::{fs, path::PathBuf, process::Command};

fn main() {
	if std::env::var_os("CARGO_FEATURE_SSR").is_none() {
		return;
	}
	let src = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");
	let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
	println!("cargo:rerun-if-changed={}", src.display());
	fs::write(out.join("uikit-classes.txt"), ev_lib_classes::CLASS_INVENTORY).unwrap();
	fs::write(out.join("tokens.css"), ev_lib_classes::TOKENS_CSS).unwrap();
	// no preflight: `map.css` hand-styles everything the uikit does not, and a reset would restyle it.
	// Layered, so that its bare element rules lose to every utility rather than beat them
	let entry = format!(
		"@layer theme, map, utilities;\n@import \"tailwindcss/theme.css\" layer(theme);\n@import \"tailwindcss/utilities.css\" layer(utilities);\n@import \"./tokens.css\";\n@import {:?} layer(map);\n@source \"./uikit-classes.txt\";\n@source {:?};\n",
		src.join("map.css").display().to_string(),
		src.join("*.rs").display().to_string(),
	);
	fs::write(out.join("entry.css"), entry).unwrap();
	// cwd is OUT_DIR so tailwind's own source detection finds nothing beyond what is named above
	let status = Command::new("tailwindcss")
		.current_dir(&out)
		.args(["-i", "entry.css", "-o", "style.css", "--minify"])
		.status()
		.expect("tailwindcss (v4) is on PATH: the dev shell, the flake's apps and the nix build carry it");
	assert!(status.success(), "tailwindcss failed on {}", out.join("entry.css").display());
}
