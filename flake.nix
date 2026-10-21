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
        pre-commit-check = pre-commit-hooks.lib.${system}.run (v_flakes.files.preCommit { inherit pkgs; stripClaudeSignature = true; });
        pname = (pkgs.lib.importTOML ./gmaps_optimal_placement/Cargo.toml).package.name;
        # inherited from the workspace, so it is not in the member's own `[package]`
        version = (pkgs.lib.importTOML ./Cargo.toml).workspace.package.version;
        stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.stdenv;
        port = 8731;

        # Pinned to match the workspace's `wasm-bindgen` — nixpkgs ships a different minor, and a
        # CLI/crate schema skew is a hard error at `wasm-bindgen` time. Shadows
        # `pkgs.wasm-bindgen-cli` (a `let` binding wins over `with pkgs;`) wherever it is referenced.
        wasm-bindgen-cli =
          let
            src = pkgs.fetchCrate {
              pname = "wasm-bindgen-cli";
              version = "0.2.126";
              hash = "sha256-H6Is3fiZVxZCfOMWK5dWMSrtn50VGv0sfdnsT+cTtyk=";
            };
          in
          pkgs.buildWasmBindgenCli {
            inherit src;
            cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
              inherit src;
              inherit (src) pname version;
              hash = "sha256-VucqkXbCi4qtQzY/HrXiDnbSURsagPsdNVMn1Tw3UiY=";
            };
          };

        # `wasm32-unknown-unknown` is already in the canonical toolchain, so the client needs no
        # target of its own here.
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

        # The server binary and the wasm client are separate compilations (`ssr` vs `hydrate`), so
        # this is the one place that knows how to produce both. Sourced by every app below.
        client = ''
          cargo build -p gmaps_optimal_placement_web --target wasm32-unknown-unknown --features hydrate --no-default-features
          mkdir -p target/site/pkg
          wasm-bindgen --target web --out-dir target/site/pkg --out-name gmaps_optimal_placement_web \
            target/wasm32-unknown-unknown/debug/gmaps_optimal_placement_web.wasm
        '';

        # The smoke test asserts Clermont's numbers, so it takes no study: `smoke.js` and
        # `car_detailing.nix` x `Clermont-Ferrand.nix` are one fixture. It is served the whole
        # product rather than that pairing, because reaching the pairing through the picker is
        # itself one of the things under test.
        study = pkgs.writeShellApplication {
          name = "study";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold nodejs chromium psmisc nix curl wasm-bindgen-cli ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            ${client}
            fuser -k ${toString port}/tcp 2>/dev/null || true
            # the smoke promotes and drops pins, so it gets a data dir of its own
            XDG_DATA_HOME="$(mktemp -d)"
            export XDG_DATA_HOME
            cargo build -p gmaps_optimal_placement # a cold compile outlasts any wait for the port
            cargo run -p gmaps_optimal_placement -- --sweep-root 1 serve --port ${toString port} & # the numbers smoke.js asserts were swept from the whole frame
            server=$!
            trap 'kill $server 2>/dev/null || true; rm -rf "$XDG_DATA_HOME"' EXIT
            up=
            for _ in $(seq 120); do
              curl -sf -o /dev/null "http://localhost:${toString port}/pkg/gmaps_optimal_placement_web.js" && { up=1; break; }
              sleep 1
            done
            [ -n "$up" ] || { echo "server never came up" >&2; exit 1; }
            node examples/smoke.js "http://localhost:${toString port}/"
          '';
        };
        # `open [-t <trades>] [-l <locations>]` — build the client, serve, open a browser. The axes
        # are the binary's own flags; everything here does is build the client and hold the port.
        # Paths are read from the repository root, which is where the defaults live.
        open = pkgs.writeShellApplication {
          name = "open-map";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold psmisc xdg-utils nix wasm-bindgen-cli ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            ${client}
            fuser -k ${toString port}/tcp 2>/dev/null || true
            exec cargo run -p gmaps_optimal_placement -- serve "$@" --port ${toString port} --open
          '';
        };
        # `searches [-t <trades>] [-l <locations>]` — build the volume chart, serve it, open it.
        searches = pkgs.writeShellApplication {
          name = "open-searches";
          runtimeInputs = with pkgs; [ rust git pkg-config openssl mold python3 psmisc xdg-utils nix fzf ];
          text = ''
            cd "$(git rev-parse --show-toplevel)"
            out="''${GMAPS_OPTIMAL_PLACEMENT_WORK:-tmp/geo}/out"
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
            out="''${GMAPS_OPTIMAL_PLACEMENT_WORK:-tmp/geo}/out/misc-$1-$2.html"
            cargo run -p gmaps_optimal_placement -- misc "$@" --out "$out"
            xdg-open "$out"
          '';
        };

        help = pkgs.writeShellApplication {
          name = "help";
          text = ''
            cat <<'EOF'
            nix run .#open -- [-t <trades>] [-l <locations>]  serve the map on :${toString port} and open it
                                            (Ctrl-C to stop). the two directories (default
                                            examples/trades examples/locations) are all this takes —
                                            the page picks the pairing, trade and city side by side,
                                            and `t` opens another as a tab over the same map
            nix run .#searches -- [-t <trades>] [-l <locations>]  open the monthly search-volume chart for
                                            the query groups of one pairing. each axis is picked through fzf
            nix run .#misc <country> <tally>  which towns in a country to write a study about at all:
                                            pool | building | parcel per --per column, --floor to skip hamlets
            nix run .#study                 serve the Clermont map and assert it in headless Chromium
            nix run .#help                  this
            cached archives and API responses live under $GMAPS_OPTIMAL_PLACEMENT_WORK (default tmp/geo);
            promoted candidates live under $XDG_DATA_HOME/gmaps_optimal_placement
            a study is a trade applied to a location. examples/trades/<t>.nix is a function of the
            location's attrset; examples/locations/<l>.nix carries area, candidate and searches.place.
            together they evaluate to what `gmaps_optimal_placement schema` describes.
            EOF
          '';
        };
      in
      {
        apps = {
          default = { type = "app"; program = pkgs.lib.getExe study; };
          study = { type = "app"; program = pkgs.lib.getExe study; };
          open = { type = "app"; program = pkgs.lib.getExe open; };
          searches = { type = "app"; program = pkgs.lib.getExe searches; };
          misc = { type = "app"; program = pkgs.lib.getExe misc; };
          help = { type = "app"; program = pkgs.lib.getExe help; };
        };

        packages.default =
          (pkgs.makeRustPlatform { rustc = rust; cargo = rust; inherit stdenv; }).buildRustPackage {
            inherit pname;
            inherit version;
            buildInputs = with pkgs; [ openssl.dev ];
            nativeBuildInputs = with pkgs; [ pkg-config wasm-bindgen-cli binaryen ];
            cargoLock.lockFile = ./Cargo.lock;
            src = pkgs.lib.cleanSource ./.;

            # cargo-leptos is bypassed: the two halves are plain cargo invocations, and this build has
            # to place the client where `LEPTOS_SITE_ROOT` will point at runtime.
            buildPhase = ''
              runHook preBuild
              cargo build --release -p gmaps_optimal_placement --bin ${pname}
              cargo build --release -p gmaps_optimal_placement_web --lib --target wasm32-unknown-unknown --features hydrate --no-default-features
              mkdir -p target/site/pkg
              wasm-bindgen --target web --out-dir target/site/pkg --out-name gmaps_optimal_placement_web \
                target/wasm32-unknown-unknown/release/gmaps_optimal_placement_web.wasm
              wasm-opt -Oz target/site/pkg/gmaps_optimal_placement_web_bg.wasm -o target/site/pkg/gmaps_optimal_placement_web_bg.wasm
              runHook postBuild
            '';

            installPhase = ''
              runHook preInstall
              mkdir -p $out/bin $out/share/${pname}
              cp target/release/${pname} $out/bin/${pname}-unwrapped
              cp -r target/site $out/share/${pname}/site
              cat > $out/bin/${pname} <<EOF
              #!${pkgs.runtimeShell}
              export LEPTOS_SITE_ROOT="\''${LEPTOS_SITE_ROOT:-$out/share/${pname}/site}"
              exec "$out/bin/${pname}-unwrapped" "\$@"
              EOF
              chmod +x $out/bin/${pname}
              runHook postInstall
            '';

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

                # cargo-leptos must match the leptos crate, so it cannot come from nixpkgs. Only
                # `cargo leptos watch` wants it — the builds above are plain cargo.
                LOCK_FILE="/tmp/cargo-leptos-install-$(echo "$PWD" | md5sum | cut -d' ' -f1).lock"
                if mkdir "$LOCK_FILE" 2>/dev/null; then
                  trap "rmdir '$LOCK_FILE' 2>/dev/null" EXIT
                  cargo install cargo-leptos
                fi
              '';

            packages = [
              binaryen
              chromium
              fzf
              mold
              nodejs
              openssl
              pkg-config
              python3
              (v_flakes.qlty system)
              rust
              study
              wasm-bindgen-cli
            ] ++ pre-commit-check.enabledPackages ++ combined.enabledPackages;

            env.RUST_BACKTRACE = 1;
          };
      }
    );
}
