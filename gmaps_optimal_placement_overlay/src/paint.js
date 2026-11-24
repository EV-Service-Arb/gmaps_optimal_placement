// Everything gmaps_optimal_placement paints over a `google.maps.Map` someone else owns: the heat, the
// pressure field and the competitor dots. Every number comes from Rust; this projects and fills,
// because that needs `fromLatLngToDivPixel` every frame. The map, its styles and its own markers are
// the host's and are never touched.

/// Per iso-line level, low to high: the line's alpha in the haze's colour, and its width. Each sits on
/// a pale halo so it reads over either end of the heat.
const LINES = [[0.75, 1.2], [0.9, 1.8], [1, 2.6]];
const STYLE = 'gop-paint-style';
let attached = 0;

/// `onView(west, south, east, north)` on every settle, and once now if the map already has bounds;
/// `onHover(lat, lng)` answers the hint's text, or nothing; `onMark(lat, lng)` on a right click.
export function attach(map, onView, onPoi, onHover, onMark) {
	if (attached++ === 0) {
		const st = document.createElement('style');
		st.id = STYLE;
		st.textContent = '.gop-nrev{text-shadow:0 0 3px #000,0 0 2px #000;font-variant-numeric:tabular-nums}';
		document.head.appendChild(st);
	}
	const canvas = blur => {
		const cv = document.createElement('canvas');
		return { cv, ctx: cv.getContext('2d'), blur };
	};
	const s = {
		map, onPoi,
		heat: canvas(0), haze: canvas(3), lines: canvas(0),
		ringX: null, ringY: null, colors: null, shown: null, tint: '', alpha: null, segs: null, opacity: 0.62, dead: false,
		markers: [], iw: new google.maps.InfoWindow(), hint: document.createElement('div'),
	};
	// `all` first: the host's own CSS may style a div
	s.hint.style.cssText = 'all:initial;display:none;position:absolute;z-index:1;pointer-events:none;white-space:pre;background:#0e1116f2;color:#e8e8ea;border:1px solid #333a47;border-radius:6px;padding:6px 8px;font:11.5px/1.45 system-ui,sans-serif';
	s.hint.className = 'gop-hint';
	map.getDiv().appendChild(s.hint);
	s.overlay = new google.maps.OverlayView();
	s.overlay.onAdd = function () {
		const pane = this.getPanes().overlayLayer;
		for (const c of [s.heat, s.haze, s.lines]) pane.appendChild(c.cv);
	};
	s.overlay.onRemove = () => { for (const c of [s.heat, s.haze, s.lines]) c.cv.remove(); };
	s.overlay.draw = () => paint(s);
	s.overlay.setMap(map);

	const bounds = () => {
		const b = !s.dead && map.getBounds();
		if (b) onView(b.getSouthWest().lng(), b.getSouthWest().lat(), b.getNorthEast().lng(), b.getNorthEast().lat());
	};
	s.on = [
		map.addListener('idle', bounds),
		map.addListener('mousemove', e => {
			const text = onHover(e.latLng.lat(), e.latLng.lng());
			if (text === undefined) return unhint(s);
			const r = map.getDiv().getBoundingClientRect();
			s.hint.textContent = text;
			s.hint.style.left = `${e.domEvent.clientX - r.left + 14}px`;
			s.hint.style.top = `${e.domEvent.clientY - r.top + 14}px`;
			s.hint.style.display = 'block';
		}),
		map.addListener('mouseout', () => unhint(s)),
		map.addListener('contextmenu', e => {
			e.domEvent.preventDefault();
			onMark(e.latLng.lat(), e.latLng.lng());
		}),
	];
	// a map that has already settled will not say so again until it moves
	queueMicrotask(bounds);
	return s;
}

export function detach(s) {
	s.dead = true;
	for (const l of s.on) l.remove();
	s.hint.remove();
	s.overlay.setMap(null);
	for (const m of s.markers) m.setMap(null);
	s.iw.close();
	if (--attached === 0) document.getElementById(STYLE).remove();
}

// copied: a view into wasm memory goes stale the moment that memory grows
export function geometry(s, ringX, ringY) {
	Object.assign(s, { ringX: ringX.slice(), ringY: ringY.slice(), colors: null, shown: null, alpha: null, segs: null });
	paint(s);
}

export function cells(s, colors, shown) {
	Object.assign(s, { colors: colors.slice(), shown: shown.slice() });
	paint(s);
}

export function field(s, tint, alpha, segs) {
	Object.assign(s, { tint, alpha: alpha.slice(), segs: segs.slice() });
	paint(s);
}

export function opacity(s, v) {
	s.opacity = v;
	paint(s);
}

function unhint(s) {
	s.hint.style.display = 'none';
}

function paint(s) {
	unhint(s); // what is under the cursor moved, or what is said of it did
	const proj = s.overlay.getProjection(), b = s.map.getBounds();
	if (!proj || !b) return;
	// scale from two known world points: robust to fractional zoom
	const p0 = proj.fromLatLngToDivPixel(new google.maps.LatLng(0, 0));
	const p1 = proj.fromLatLngToDivPixel(new google.maps.LatLng(0, 90));
	const k = (p1.x - p0.x) / 64, ox = p0.x - 128 * k, oy = p0.y - 128 * k;
	const nw = proj.fromLatLngToDivPixel(new google.maps.LatLng(b.getNorthEast().lat(), b.getSouthWest().lng()));
	const el = s.map.getDiv(), W = el.offsetWidth, H = el.offsetHeight, dpr = window.devicePixelRatio || 1;
	for (const c of [s.heat, s.haze, s.lines]) {
		if (c.cv.width !== W * dpr || c.cv.height !== H * dpr) { c.cv.width = W * dpr; c.cv.height = H * dpr; }
		c.cv.style.cssText = `position:absolute;pointer-events:none;left:${nw.x}px;top:${nw.y}px;width:${W}px;height:${H}px${c.blur ? `;filter:blur(${c.blur}px)` : ''}`;
		c.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
		c.ctx.clearRect(0, 0, W, H);
	}
	const X = i => ox + s.ringX[i] * k - nw.x, Y = i => oy + s.ringY[i] * k - nw.y;
	const quads = (ctx, on, fill) => {
		for (let i = 0; i < on.length; i++) {
			if (!on[i]) continue;
			const x0 = X(i * 4), y0 = Y(i * 4);
			if (x0 < -40 || y0 < -40 || x0 > W + 40 || y0 > H + 40) continue;
			ctx.fillStyle = fill(i);
			ctx.beginPath(); ctx.moveTo(x0, y0);
			for (let c = 1; c < 4; c++) ctx.lineTo(X(i * 4 + c), Y(i * 4 + c));
			ctx.closePath(); ctx.fill();
		}
	};
	if (s.shown && s.shown.length) {
		s.heat.ctx.globalAlpha = s.opacity;
		quads(s.heat.ctx, s.shown, i => `rgb(${s.colors[i * 3]},${s.colors[i * 3 + 1]},${s.colors[i * 3 + 2]})`);
		s.heat.ctx.globalAlpha = 1;
	}
	if (s.alpha && s.alpha.length) {
		quads(s.haze.ctx, s.alpha, i => `rgba(${s.tint},${s.alpha[i] / 255})`);
		const ctx = s.lines.ctx;
		ctx.lineJoin = ctx.lineCap = 'round';
		for (let l = 0; l < LINES.length; l++) {
			ctx.beginPath();
			for (let j = 0; j < s.segs.length; j += 5) {
				if (s.segs[j + 4] !== l) continue;
				ctx.moveTo(ox + s.segs[j] * k - nw.x, oy + s.segs[j + 1] * k - nw.y);
				ctx.lineTo(ox + s.segs[j + 2] * k - nw.x, oy + s.segs[j + 3] * k - nw.y);
			}
			ctx.strokeStyle = 'rgba(255,255,255,.6)';
			ctx.lineWidth = LINES[l][1] + 2;
			ctx.stroke();
			ctx.strokeStyle = `rgba(${s.tint},${LINES[l][0]})`;
			ctx.lineWidth = LINES[l][1];
			ctx.stroke();
		}
	}
}

/// The whole competitor set, replacing whatever is there: each its size and whether it shows decided
/// in Rust. A click opens the info window, which reads what it scored on each search; the hooks,
/// where the host gave them, also hear which dot it was.
export function dots(s, json) {
	for (const m of s.markers) m.setMap(null);
	s.markers = JSON.parse(json).map((c, i) => {
		const m = new google.maps.Marker({
			position: { lat: c.lat, lng: c.lng }, zIndex: c.big ? 3 : 2, map: c.show ? s.map : null,
			label: c.n_rev ? { text: String(c.n_rev), className: 'gop-nrev', color: '#fff', fontSize: '10px', fontWeight: '700' } : null,
			icon: {
				path: google.maps.SymbolPath.CIRCLE, fillColor: c.color, fillOpacity: 0.95,
				strokeColor: '#111', strokeWeight: 1.2, scale: c.scale,
				// labelOrigin is in path units, so divide out the scale to keep the gap constant in pixels
				labelOrigin: new google.maps.Point(0, -(c.scale + 8) / c.scale),
			},
		});
		m.addListener('click', () => {
			const searches = c.searches.map(q => `<tr style="color:${q.on ? '#111' : '#999'}"><td>${esc(q.text)}</td><td style="text-align:right;padding-left:8px">${esc(q.w)}</td></tr>`).join('');
			s.iw.setContent(`<div style="font:13px system-ui;max-width:250px;color:#111">
				<b>${esc(c.name)}</b><br>${esc(c.addr)}<br>
				<span style="color:#555">${esc(c.kind)} · ${esc(c.tier)}</span><br>
				★ ${c.rating ?? '–'} (${c.n_rev} reviews)${c.tel ? '<br>' + esc(c.tel) : ''}
				${c.web ? `<br><a href="${esc(c.web)}" target="_blank" rel="noreferrer">website</a>` : ''}
				<table style="margin-top:4px;font-size:12px;border-collapse:collapse"><tr><td colspan="2" style="color:#555">weight ${esc(c.w)}, by search</td></tr>${searches}</table></div>`);
			s.iw.open(s.map, m);
			if (s.onPoi) s.onPoi(i);
		});
		return m;
	});
}

const esc = t => String(t ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
