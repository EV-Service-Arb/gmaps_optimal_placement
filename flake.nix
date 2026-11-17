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
        pname = (pkgs.lib.importTOML ./gmaps_optimal_placement/Cargo.toml).package.name;
        # inherited from the workspace, so it is not in the member's own `[package]`
        version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
        port = 8731;
        # the first of this repository's ten in SA_PORT_RANGE: site_conductor's docs/port_range.md
        SA_PORTS = 52850;

        rs = v_flakes.rs { inherit pkgs rust; };
        github = v_flakes.github {
          inherit pkgs pname rs;
          enable = true;
          lastSupportedVersion = "nightly-2026-07-14";
          jobs.default = true;
          lfs = false;
          labels.extra = [
            { name = "reviews"; color = "0000ff"; description = "Anythin pertaining to improving review scoring"; }
          ];

        };
        readme = v_flakes.readme-fw {
          inherit pkgs pname;
          defaults = true;
          lastSupportedVersion = "nightly-1.92";
          rootDir = ./.;
          badges = [ "msrv" "loc" "ci" ];
        };
        combined = v_flakes.utils.combine { inherit rust; modules = [ rs github readme ]; };

        # `conductor --port <p> [-t <trades>] [--countries <dir>]` — the conductor's peer: the studies
        # behind its assertion. Run from this checkout; needs PANEL_ASSERTION_KEYS, and the config's
        # keys for what a pull buys under `--spend-real-quota-i-am-certain` (without it a pull is refused).
        conductor = pkgs.writeShellApplication {
          name = "conductor";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold nix ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            exec cargo run -p gmaps_optimal_placement -- serve "$@"
          '';
        };
        # `searches [-c <category>] [-l <locations>]` — build the volume chart, serve it, open it.
        searches = pkgs.writeShellApplication {
          name = "open-searches";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold python3 psmisc xdg-utils nix fzf ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            out="''${XDG_CACHE_HOME:-$HOME/.cache}/gmaps_optimal_placement/out"
            cargo run -p gmaps_optimal_placement -- searches "$@" --out "$out/searches.html"
            fuser -k ${toString port}/tcp 2>/dev/null || true
            xdg-open "http://localhost:${toString port}/searches.html" &
            cd "$out" && python3 -m http.server ${toString port}
          '';
        };

        # `misc <country> <tally> [--per col] [--floor n]` — one map of a whole country, no study.
        # Written per country and tally, so two denominators of the same tally overwrite each other.
        misc = pkgs.writeShellApplication {
          name = "misc-map";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold xdg-utils nix ];
          text = ''
            [ $# -ge 2 ] || { echo "usage: nix run .#misc <country> <tally> [--per <column>] [--floor <n>]" >&2; exit 1; }
            cd "$(git rev-parse --show-toplevel)"
            out="''${XDG_CACHE_HOME:-$HOME/.cache}/gmaps_optimal_placement/out/misc-$1-$2.html"
            cargo run -p gmaps_optimal_placement -- misc "$@" --out "$out"
            xdg-open "$out"
          '';
        };

        help = pkgs.writeShellApplication {
          name = "help";
          text = ''
            cat <<'EOF'
            nix run .#searches -- [-c <category>] [-l <locations>]  open the monthly search-volume chart for
                                            the query groups of one pairing. each axis is picked through fzf
            nix run .#misc <country> <tally>  which towns in a country to write a study about at all:
                                            pool | building | parcel per --per column, --floor to skip hamlets
            nix run .#conductor -- --port <p> [-t ..] [--countries ..]  serve the conductor the studies it
                                            hands in, behind its assertion (PANEL_ASSERTION_KEYS); only a pull spends
            nix run .#help                  this
            cached archives and API responses live under $XDG_CACHE_HOME/gmaps_optimal_placement
            a study is a category over an area. the conductor's category names the trade that
            studies it; examples/trades/<t>.nix is a function of the area's attrset;
            examples/locations/<l>.nix carries area, candidate and searches.place over
            examples/countries/<CC>.nix. together they evaluate to what `gmaps_optimal_placement schema`
            describes.
            EOF
          '';
        };
      in
      {
        apps = {
          default = { type = "app"; program = pkgs.lib.getExe conductor; };
          conductor = { type = "app"; program = pkgs.lib.getExe conductor; };
          searches = { type = "app"; program = pkgs.lib.getExe searches; };
          misc = { type = "app"; program = pkgs.lib.getExe misc; };
          help = { type = "app"; program = pkgs.lib.getExe help; };
        };

        packages.default =
          (pkgs.makeRustPlatform { rustc = build_rust; cargo = build_rust; inherit stdenv; }).buildRustPackage {
            inherit pname;
            inherit version;
            buildInputs = with pkgs; [ openssl.dev ];
            nativeBuildInputs = with pkgs; [ pkg-config ];
            cargoLock = {
              lockFile = ./Cargo.lock;
              allowBuiltinFetchGit = true;
            };
            src = pkgs.lib.cleanSource ./.;
            cargoBuildFlags = [ "-p" "gmaps_optimal_placement" ];
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
              fzf
              mold
              openssl
              pkg-config
              python3
              (v_flakes.qlty system)
              rust
            ] ++ pre-commit-check.enabledPackages ++ combined.enabledPackages;

            env.RUST_BACKTRACE = 1;
            # where the conductor's local-stack serves `conductor` from
            PLACEMENT_PORT = toString SA_PORTS;
          };
      }
    );
}
