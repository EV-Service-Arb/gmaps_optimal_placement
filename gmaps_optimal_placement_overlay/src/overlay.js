// The module a host page `import()`s: `overlay(map, {api, csrf})`. Its wasm is fetched relative to
// this file, so it works under whatever prefix it is served from.
import init, { Overlay } from './gmaps_optimal_placement_overlay.js';

export async function overlay(map, { api, csrf }) {
	await init();
	return new Handle(map, api, csrf);
}

class Handle extends EventTarget {
	#o;

	constructor(map, api, csrf) {
		super();
		this.#o = new Overlay(map, api, csrf, (type, detail) => this.dispatchEvent(new CustomEvent(type, { detail: JSON.parse(detail) })));
	}

	show(id) { return this.#o.show(id); }
	heat(layerId) { this.#o.heat(layerId); }
	tune(t) { this.#o.tune(JSON.stringify(t)); }
	pressure(on) { this.#o.pressure(on); }
	competitors(on) { this.#o.competitors(on); }
	opacity(x) { this.#o.opacity(x); }
	hideImputed(on) { this.#o.hideImputed(on); }
	pull() { return this.#o.pull(); }

	dispose() {
		this.#o.dispose();
		this.#o.free();
		this.#o = null;
	}
}
