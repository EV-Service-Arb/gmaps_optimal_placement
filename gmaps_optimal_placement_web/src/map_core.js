// The map instance and gop's own pins. What is painted over the map — the heat, the pressure field,
// the competitor dots — is `gmaps_optimal_placement_overlay`'s, the same code the conductor loads;
// `mount` hands it the map.
//
// Nothing throws across the boundary: under `panic=abort` a rejected promise reaching wasm kills the
// app, so `mount` returns a banner string instead of the map when it fails.

const S = new WeakMap();

/// `shell()` resolves this once the Maps bootstrap has run.
const ready = () => window.__mapsReady ?? Promise.reject(new Error('the Maps bootstrap never ran'));

export async function mount(el, lat, lng, zoom, onClick, onMove, onOut, onPin) {
	try {
		if (S.has(el)) throw new Error('the map is already mounted');
		await ready();
		const map = new google.maps.Map(el, {
			center: { lat, lng }, zoom, mapTypeId: 'roadmap',
			streetViewControl: false, fullscreenControl: false, clickableIcons: false,
			styles: [{ featureType: 'poi', elementType: 'labels', stylers: [{ visibility: 'off' }] }],
		});
		S.set(el, { map, onPin, pins: new Map() });
		map.addListener('click', e => onClick(e.latLng.lat(), e.latLng.lng()));
		map.addListener('mousemove', e => {
			const r = el.getBoundingClientRect();
			onMove(e.latLng.lat(), e.latLng.lng(), e.domEvent.clientX - r.left, e.domEvent.clientY - r.top);
		});
		map.addListener('mouseout', () => onOut());
		return map;
	} catch (e) {
		return `⚠ map failed to load — ${(e && e.message) || e}`;
	}
}

/// Where a tab switch puts the viewport: the new study's own frame. A move to where the map already
/// is starts a gesture that swallows the next click, so it is not made.
export function recenter(el, lat, lng, zoom) {
	const s = S.get(el);
	if (!s) return;
	const c = s.map.getCenter();
	if (s.map.getZoom() === zoom && c && Math.abs(c.lat() - lat) < 1e-9 && Math.abs(c.lng() - lng) < 1e-9) return;
	s.map.setCenter({ lat, lng });
	s.map.setZoom(zoom);
}

/// Where a found place is, at the zoom already chosen.
export function pan(el, lat, lng) {
	const s = S.get(el);
	if (!s) return;
	s.map.panTo({ lat, lng });
}

/// The full pin set, replacing whatever is there. Every pin is clickable, whether or not it is
/// lettered — an unlabelled pin you cannot select is a pin you cannot delete.
export function pins(el, json) {
	const s = S.get(el);
	if (!s) return;
	for (const m of s.pins.values()) m.setMap(null);
	s.pins.clear();
	for (const p of JSON.parse(json)) {
		const m = new google.maps.Marker({
			position: { lat: p.lat, lng: p.lng }, map: s.map, zIndex: 9, title: p.title ?? '',
			label: p.label ? { text: p.label, color: '#000', fontSize: '11px', fontWeight: '700' } : null,
			icon: {
				path: google.maps.SymbolPath.CIRCLE, fillColor: p.color, fillOpacity: 1,
				strokeColor: '#000', strokeWeight: 1.5, scale: 11,
			},
		});
		m.addListener('click', () => s.onPin(p.id));
		s.pins.set(p.id, m);
	}
}
