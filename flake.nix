{
  inputs = {
    v_flakes.url = "github:valeratrades/v_flakes?ref=v1.6";
  };
  outputs = { self, v_flakes }:
    let
      inherit (v_flakes) flake-utils pre-commit-hooks;
    in
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import v_flakes.default_nixpkgs { inherit system; config.allowUnfree = true; };
        rust = v_flakes.rs.default_nightly system;
        build_rust = v_flakes.rs.build_nightly system;
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v_flakes.files.preCommit { inherit pkgs; stripClaudeSignature = true; });
        # the CLI's binary, which keeps the library's name
        pname = "gmaps_optimal_placement";
        # inherited from the workspace, so it is not in the member's own `[package]`
        version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
        lib = pkgs.lib;
        # the first of this repository's ten in SA_PORT_RANGE: site_conductor's docs/port_range.md
        SA_PORTS = 52850;
        rustPlatform = pkgs.makeRustPlatform { rustc = build_rust; cargo = build_rust; inherit stdenv; };
        cargoLock = {
          lockFile = ./Cargo.lock;
          allowBuiltinFetchGit = true;
        };

        rs = v_flakes.rs { inherit pkgs rust; pinDeps = true; };
        oldestSupportedVersion = "nightly-2026-06-29";
        newestSupportedNightly = "nightly-2026-07-14";
        github = v_flakes.github {
          inherit pkgs pname rs;
          enable = true;
          inherit oldestSupportedVersion; inherit newestSupportedNightly;
          jobs.default = true;
          lfs = false;
          labels.extra = [
            { name = "reviews"; color = "0000ff"; description = "Anythin pertaining to improving review scoring"; }
          ];

        };
        readme = v_flakes.readme-fw {
          inherit pkgs pname;
          defaults = true;
          inherit oldestSupportedVersion; inherit newestSupportedNightly;
          rootDir = ./.;
          badges = [ "msrv" "newest_nightly" "loc" "ci" ];
        };
        combined = v_flakes.utils.combine { inherit rust; modules = [ rs github readme ]; };

        # `misc <country> <tally> [--per col] [--floor n]` — one map of a whole country, no study.
        # Written per country and tally, so two denominators of the same tally overwrite each other.
        misc = pkgs.writeShellApplication {
          name = "misc-map";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold xdg-utils tailwindcss_4 ];
          text = ''
            [ $# -ge 2 ] || { echo "usage: nix run .#misc <country> <tally> [--per <column>] [--floor <n>]" >&2; exit 1; }
            cd "$(git rev-parse --show-toplevel)"
            out="''${XDG_CACHE_HOME:-$HOME/.cache}/gmaps_optimal_placement/out/misc-$1-$2.html"
            cargo run -p gmaps_optimal_placement_cli -- misc "$@" --out "$out"
            xdg-open "$out"
          '';
        };

        # The CLI must be the lock's `wasm-bindgen`: a skew is a hard error at bindgen time.
        wasmBindgen = (lib.findFirst (p: p.name == "wasm-bindgen") (throw "Cargo.lock holds no wasm-bindgen") (lib.importTOML ./Cargo.lock).package).version;
        wasm-bindgen-cli =
          let cli = pkgs."wasm-bindgen-cli_${lib.replaceStrings [ "." ] [ "_" ] wasmBindgen}" or (throw "nixpkgs has no wasm-bindgen-cli ${wasmBindgen}, the lock's wasm-bindgen"); in
          assert lib.assertMsg (cli.version == wasmBindgen) "wasm-bindgen-cli ${cli.version} is not the lock's wasm-bindgen ${wasmBindgen}";
          cli;
        # gmaps_optimal_placement_overlay as a page imports it: wasm-bindgen's module, its .d.ts, the wasm
        overlay = rustPlatform.buildRustPackage {
          pname = "gmaps_optimal_placement_overlay";
          inherit version cargoLock;
          src = pkgs.lib.cleanSource ./.;
          nativeBuildInputs = [ wasm-bindgen-cli pkgs.binaryen ];
          buildPhase = ''
            runHook preBuild
            cargo build --release -p gmaps_optimal_placement_overlay --target wasm32-unknown-unknown
            wasm-bindgen --target web --out-dir "$out" --out-name overlay target/wasm32-unknown-unknown/release/gmaps_optimal_placement_overlay.wasm
            wasm-opt -Oz "$out/overlay_bg.wasm" -o "$out/overlay_bg.wasm"
            runHook postBuild
          '';
          installPhase = "true";
          doCheck = false;
          auditable = false; # cargo-auditable doesn't support edition 2024
        };
        # The server (`ssr`, inside the CLI) and the wasm client (`hydrate`) are separate compilations;
        # this is the one place that knows how to produce the client, under `site`'s pkg/.
        client = site: ''
          cargo build -p gmaps_optimal_placement_web --lib --target wasm32-unknown-unknown --no-default-features --features hydrate
          wasm-bindgen --target web --out-dir ${site}/pkg --out-name gmaps_optimal_placement_web \
            target/wasm32-unknown-unknown/debug/gmaps_optimal_placement_web.wasm
        '';
        # `view [-c <category>]` — the GUI, from this checkout
        view = pkgs.writeShellApplication {
          name = "view";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold wasm-bindgen-cli tailwindcss_4 ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            ${client "target/site"}
            exec cargo run -p gmaps_optimal_placement_cli -- view --port ${toString SA_PORTS} --site target/site "$@"
          '';
        };
        # The GUI in headless Chromium: the picker, one pairing loaded, the map painted. Served on the
        # range's second port out of a site of its own, so a GUI already up is left alone.
        smoke = pkgs.writeShellApplication {
          name = "smoke";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold wasm-bindgen-cli tailwindcss_4 nodejs chromium curl ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            ${client "target/smoke"}
            cargo build -p gmaps_optimal_placement_cli # a cold compile outlasts any wait for the port
            port=${toString (SA_PORTS + 1)}
            cargo run -p gmaps_optimal_placement_cli -- view --port "$port" --site target/smoke "$@" &
            server=$!
            trap 'kill $server 2>/dev/null || true' EXIT
            up=
            for _ in $(seq 120); do
              curl -sf -o /dev/null "http://127.0.0.1:$port/pkg/gmaps_optimal_placement_web.js" && { up=1; break; }
              sleep 1
            done
            [ -n "$up" ] || { echo "server never came up" >&2; exit 1; }
            node examples/smoke.js "http://127.0.0.1:$port/"
          '';
        };
        help = pkgs.writeShellApplication {
          name = "help";
          text = ''
            cat <<'EOF'
            nix run -- [-c <category>]      the GUI on 127.0.0.1:${toString SA_PORTS}: every area the work dir's
                                            studies are over, under every category they carry, or on its own;
                                            category and location are picked side by side, `t` opens another
                                            as a tab over the same map. Buys nothing: what is not held is named
            nix run .#smoke                 the GUI in headless Chromium: pick a pairing, see the map paint
            nix run .#misc <country> <tally>  which towns in a country to write a study about at all:
                                            pool | building | parcel per --per column, --floor to skip hamlets
            nix run .#help                  this
            cached archives and API responses live under $XDG_CACHE_HOME/gmaps_optimal_placement
            a study is the conductor's category over the conductor's area, on the country's base
            compiled into the library, which `gmaps_optimal_placement schema base` describes.
            EOF
          '';
        };
      in
      {
        apps = {
          default = { type = "app"; program = pkgs.lib.getExe view; };
          smoke = { type = "app"; program = pkgs.lib.getExe smoke; };
          misc = { type = "app"; program = pkgs.lib.getExe misc; };
          help = { type = "app"; program = pkgs.lib.getExe help; };
        };

        packages.overlay = overlay;
        packages.default =
          rustPlatform.buildRustPackage {
            inherit pname version cargoLock;
            buildInputs = with pkgs; [ openssl.dev ];
            nativeBuildInputs = with pkgs; [ pkg-config tailwindcss_4 ]; # tailwind: the web crate's build.rs
            src = pkgs.lib.cleanSource ./.;
            cargoBuildFlags = [ "-p" "gmaps_optimal_placement_cli" ];
            doCheck = false;
            auditable = false; # cargo-auditable doesn't support edition 2024
          };

        devShells.default =
          with pkgs;
          mkShell {
            inherit stdenv;
            shellHook =
              pre-commit-check.shellHook
              + combined.shellHook
              + ''
                cp -f ${(v_flakes.files.treefmt) { inherit pkgs; }} ./.treefmt.toml
                cp -f ${(v_flakes.files.gitattributes) { inherit pkgs; lfs = false; }} ./.gitattributes
              '';

            packages = [
              mold
              openssl
              pkg-config
              python3
              (v_flakes.qlty system)
              rust
              tailwindcss_4
              wasm-bindgen-cli
            ] ++ pre-commit-check.enabledPackages ++ combined.enabledPackages;

            env.RUST_BACKTRACE = 1;
          };
      }
    );
}
