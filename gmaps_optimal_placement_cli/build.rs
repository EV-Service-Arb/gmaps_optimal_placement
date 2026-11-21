fn main() {
	// v_utils names the settings, their env prefix and the work dir after the package; the CLI is the app the library is named for
	println!("cargo:rustc-env=CARGO_PKG_NAME=gmaps_optimal_placement");
}
