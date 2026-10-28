/* Drives the served map in headless Chromium over CDP. Run: node smoke.js <url>
 *
 * Only what is genuinely browser-shaped lives here: the island hydrates, the canvas actually
 * paints, the markers attach, and a pin round-trips through the server to the file under
 * XDG_DATA_HOME. The model's own numbers moved to `cargo t -p gmaps_optimal_placement_core`, which checks them
 * against the same Clermont fixture with no browser and no key.
 *
 * Writes pins, so the caller must point XDG_DATA_HOME somewhere disposable. */
const { spawn } = require("node:child_process");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");

const URL_ = process.argv[2] || "http://localhost:8731/";
const PORT = 9333;
const PINS = path.join(process.env.XDG_DATA_HOME || "", "gmaps_optimal_placement", "pins-car_detailing_-_Clermont-Ferrand.json");
const PROMOTED = "Smoke Site";

const chrome = spawn("chromium", [
  "--headless=new", "--disable-gpu", "--no-sandbox", "--window-size=1400,900",
  `--remote-debugging-port=${PORT}`, "about:blank",
], { stdio: "ignore" });

const sleep = ms => new Promise(r => setTimeout(r, ms));
// unbuffered: a hang mid-run should still show how far it got
const log = (...a) => fs.writeSync(1, a.map(x => (typeof x === "string" ? x : JSON.stringify(x))).join(" ") + "\n");
const pins = () => (fs.existsSync(PINS) ? JSON.parse(fs.readFileSync(PINS, "utf8")) : null);

async function main() {
  assert(process.env.XDG_DATA_HOME, "smoke.js writes pins — point XDG_DATA_HOME somewhere disposable");
  fs.rmSync(PINS, { force: true });

  let targets;
  for (let i = 0; i < 40; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json(); break; }
    catch { await sleep(250); }
  }
  const ws = new WebSocket(targets.find(t => t.type === "page").webSocketDebuggerUrl);
  await new Promise(r => ws.addEventListener("open", r));

  let id = 0;
  const pending = new Map();
  const errors = [];
  // how to answer the next prompt/confirm; the click that raises one cannot be awaited. `null`
  // asserts no dialog is expected at all.
  let answer = null;
  const dialogs = [];
  ws.addEventListener("message", ev => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
    if (m.method === "Runtime.exceptionThrown")
      errors.push(m.params.exceptionDetails.exception?.description || "exception");
    if (m.method === "Page.javascriptDialogOpening") {
      dialogs.push(m.params.type);
      send("Page.handleJavaScriptDialog", { accept: answer !== null, promptText: answer ?? undefined });
    }
  });
  const send = (method, params = {}) => new Promise(res => {
    const i = ++id; pending.set(i, res);
    ws.send(JSON.stringify({ id: i, method, params }));
  });
  const evaluate = async expr => {
    const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true });
    if (r.result?.exceptionDetails) throw new Error(r.result.exceptionDetails.exception.description);
    return r.result.result.value;
  };
  const act = glyph => `[...document.querySelectorAll('#report .acts button')].find(b => b.textContent.trim() === '${glyph}')`;
  const ctl = text => `[...document.querySelectorAll('#ctl button')].find(b => b.textContent.includes(${JSON.stringify(text)}))`;
  const markers = () => evaluate("document.querySelectorAll('#map [role=button]').length");
  // pins reach the map through an effect, so a count is only wrong once it has stopped changing
  const expectMarkers = async (n, msg) => {
    let got;
    for (let i = 0; i < 20; i++) { got = await markers(); if (got === n) return; await sleep(500); }
    assert.equal(got, n, msg);
  };
  // one uikit `Command` per axis; a row's place on screen is its CSS `order`, which is how it is ranked
  const col = n => `document.querySelectorAll('#picker [data-slot=command]')[${n}]`;
  const items = n => `[...${col(n)}.querySelectorAll('[data-slot=command-item]')]`;
  const rows = n => evaluate(`${items(n)}.sort((a, b) => a.style.order - b.style.order).map(i => i.textContent)`);
  const pick = (n, name) => evaluate(`${items(n)}.find(i => i.textContent === ${JSON.stringify(name)}).click()`);
  const take = () => evaluate("document.querySelector('#picker [data-slot=button]').click()");
  const picked = n => evaluate(`${col(n)}.querySelector('[data-slot=command-item]:has([aria-label=picked])')?.textContent ?? null`);
  const invalid = n => evaluate(`${col(n)}.querySelector('[data-slot=command-input]').getAttribute('aria-invalid')`);
  const focused = () => evaluate(`(() => { const a = document.activeElement;
    return a.dataset.slot === 'button' ? 'load' : [...document.querySelectorAll('#picker [data-slot=command-input]')].indexOf(a); })()`);
  const pickerUp = () => evaluate("!!document.querySelector('#picker')");
  const settled = () =>
    evaluate("document.querySelectorAll('#ctl select option').length > 0 && !!document.querySelector('canvas')").catch(() => false);
  // the panel is server-rendered empty and filled by the island, so its contents are the only
  // honest signal that the wasm landed and the payload parsed. Nothing is served prebuilt, so
  // every visit starts at the picker and the fixture pairing is reached the way any other is
  const load = async () => {
    await send("Page.navigate", { url: URL_ });
    for (let i = 0; ; i++) {
      if (await evaluate("!!document.querySelector('#picker [data-slot=command-input]')").catch(() => false)) break;
      if (i === 60) throw new Error("the picker never rendered");
      await sleep(500);
    }
    // a click picks its own field and no more, so the loading is asked for on its own
    await pick(0, "car_detailing");
    await pick(1, "Clermont-Ferrand");
    await take();
    // an uncached pairing reads the 87 MB archive and every POI page before the panel can fill
    for (let i = 0; i < 240; i++) {
      if (await settled()) return;
      await sleep(1000);
    }
    throw new Error("the island never settled");
  };
  const press = async key => {
    await send("Input.dispatchKeyEvent", { type: "keyDown", key, text: key });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key });
    await sleep(700);
  };
  // a named key carries no text, or the field it is aimed at takes the name as characters — save
  // Enter, whose "\r" is what activates a focused button
  const stroke = async (key, code) => {
    const p = { key, code: key, windowsVirtualKeyCode: code, nativeVirtualKeyCode: code };
    await send("Input.dispatchKeyEvent", key === "Enter" ? { type: "keyDown", text: "\r", ...p } : { type: "rawKeyDown", ...p });
    await send("Input.dispatchKeyEvent", { type: "keyUp", ...p });
    await sleep(700);
  };
  const tabs = () => evaluate("[...document.querySelectorAll('#tabs .tab:not(.add)')].map(t => t.textContent.replace('●',''))");
  const layers = () => evaluate("document.querySelectorAll('#ctl select option').length");
  // a click that raises a dialog blocks the page, so the evaluate must not be awaited
  const clickThrough = (expr, reply = null) => {
    answer = reply;
    dialogs.length = 0;
    send("Runtime.evaluate", { expression: `(${expr}).click()` });
    return sleep(1500);
  };

  await send("Runtime.enable");
  await send("Page.enable");
  await load();

  const s = await evaluate(`(() => {
    const cv = document.querySelector('canvas');
    return {
      layers: document.querySelectorAll('#ctl select option').length,
      tierRows: document.querySelectorAll('#ctl .row').length,
      cats: [...document.querySelectorAll('#ctl .cat')].map(d => [d.querySelector('label').textContent, d.querySelector('.sw').style.background]),
      tiered: [...document.querySelectorAll('#ctl .row')].filter(r => r.querySelector('input[id^=tier]'))
        .map(r => Number(r.querySelector('label').textContent.match(/\\((\\d+)\\)$/)[1])),
      ticks: [...document.querySelectorAll('#ticks span')].map(t => t.textContent).join(' '),
      note: document.querySelector('#ctl div.note').innerText.replace(/\\n/g, ' '),
      banner: document.querySelector('#banner')?.innerText ?? null,
      canvas: [cv.width, cv.height],
      painted: (() => { const d = cv.getContext('2d').getImageData(0, 0, cv.width, cv.height).data;
        let n = 0; for (let i = 3; i < d.length; i += 4 * 97) if (d[i] > 0) n++; return n; })(),
    };
  })()`);
  log("state:", s);

  assert.equal(s.banner, null, "no banner");
  assert.equal(s.layers, 10, "four live layers plus the study's six");
  assert.equal(s.tierRows, 3, "one row per tier, plus hide-imputed");
  assert(s.note.includes("10446 cells"), `legend counts the grid: ${s.note}`);
  assert(s.painted > 100, "canvas actually painted");
  // the colours are Google's categories, not the study's tiers, so the list is longer than `tier`
  // is and no two entries share a swatch
  assert(s.cats.length > 2, `the category list is the inventory's own: ${JSON.stringify(s.cats)}`);
  assert.equal(new Set(s.cats.map(c => c[1])).size, s.cats.length, "every category gets its own colour");
  const sum = v => v.reduce((a, b) => a + b, 0);
  assert.equal(sum(s.cats.map(c => Number(c[0].match(/\((\d+)\)$/)[1]))), sum(s.tiered), "every competitor is filed under exactly one category");
  // a marker reaches the map a frame after the panel it belongs to, so this one settles like the rest
  await expectMarkers(131, "130 competitor markers and the study's one candidate");

  log("step: hover a competitor");
  // the name, off the marker, in the same tooltip a cell uses — the native `title` is gone
  const hover = await evaluate(`(() => { const m = document.querySelectorAll('#map [role=button]')[0];
    const r = m.getBoundingClientRect();
    m.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2 }));
    return new Promise(res => setTimeout(() => res(document.getElementById('tip')?.innerText ?? null), 400)); })()`);
  log("tip:", hover);
  assert(hover, "hovering a competitor names it");

  // a click drops a pin and opens its card — the card is what makes a yellow pin reachable at all.
  // 430 rather than 400: `#tabs` pushes the map down its own height, and 400 now lands on a marker
  log("step: click the map");
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: 700, y: 430, button: "left", buttons: 1, clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 700, y: 430, button: "left", buttons: 0, clickCount: 1 });
  await sleep(1500);
  const card = await evaluate(`(() => { const el = document.getElementById('report');
    return el && { text: el.innerText.slice(0, 160), acts: [...el.querySelectorAll('.acts button')].map(b => b.textContent.trim()) }; })()`);
  log("---- card ----\n" + (card?.text ?? "(none)"));
  assert(card, "a click opens a card");
  assert.deepEqual(card.acts, ["✓", "⧉", "✕"], "a yellow pin can be kept, copied or dropped");
  await expectMarkers(132, "the clicked pin is on the map");
  assert(card.text.includes("Capture score"), "the card is a site report");

  log("step: promote");
  // ✓ — name it, and it becomes green, lettered, and a line in the file
  await clickThrough(act("✓"), PROMOTED);
  assert.deepEqual(pins()?.added?.map(p => p.name), [PROMOTED], `the promotion is in ${PINS}`);
  assert.deepEqual(pins()?.hidden, [], "nothing hidden yet");
  const promoted = await evaluate(`document.querySelector('#report h4').textContent`);
  assert(promoted.startsWith("B · " + PROMOTED), `the promotion takes the next letter: ${promoted}`);

  log("step: rank");
  // the green set survives both of the buttons that used to wipe it
  await evaluate(`${ctl("Rank top 10 sites")}.click()`);
  await expectMarkers(142, "ranking adds ten yellow pins and keeps the two green ones");
  await evaluate(`${ctl("Clear pins")}.click()`);
  await expectMarkers(132, "clearing takes the yellow pins and leaves the green ones");

  await evaluate(`${ctl("Compare")}.click()`);
  await sleep(800);
  const compare = await evaluate(`document.getElementById('report').innerText.slice(0, 250)`);
  assert(compare.includes("VifNet") && compare.includes(PROMOTED), `the comparison reads the effective set: ${compare}`);
  log("---- candidates ----\n" + compare);

  log("step: reload");
  // it is still there after a restart of the page, because the server kept it
  await load();
  await expectMarkers(132, "the promoted pin comes back green, the yellow ones do not");

  log("step: drop");
  // ✕ on a promoted pin: nothing to confirm, it was never an argument in the study file
  await clickThrough(`document.querySelectorAll('#map [role=button]')[131]`);
  assert.equal((await evaluate(`document.querySelector('#report h4')?.textContent ?? ''`)).startsWith("B · " + PROMOTED), true, "the promoted pin opens its own card");
  await clickThrough(act("✕"));
  assert.deepEqual(dialogs, [], "dropping a pin this session promoted asks nothing");
  assert.deepEqual(pins()?.added, [], "dropping a promoted pin takes it out of the file");
  await expectMarkers(131, "and off the map");

  log("step: hide a study candidate");
  // ✕ on one the study file names is an argument with the document, so it is confirmed
  await clickThrough(`document.querySelectorAll('#map [role=button]')[130]`);
  assert.equal((await evaluate(`document.querySelector('#report h4')?.textContent ?? ''`)).startsWith("A · VifNet"), true, "the study's candidate opens its own card");
  await clickThrough(act("✕"), "");
  assert.deepEqual(dialogs, ["confirm"], "hiding a candidate the study names is confirmed first");
  assert.deepEqual(pins()?.hidden, ["VifNet"], "the study file is untouched; the hiding lives beside it");
  await load();
  await expectMarkers(130, "it stays hidden across a reload");

  // the study file is still the seed: delete the diff and the candidate is back
  fs.rmSync(PINS);
  await load();
  await expectMarkers(131, "deleting the pin file restores what the study declares");

  // the CLI was given the two directories, so the pool is their whole product and the one open tab
  // is what the picker was pointed at; a pairing nobody asked for is built the first time a tab does
  log("step: tabs");
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand"], "the pairing picked at load is the one open tab");
  assert.equal(await layers(), 10, "four live layers plus the study's six");

  // the city on its own: the archive and nothing billed, which is what the last row of the trade
  // field buys. Everything a slider moves is a trade's, so the panel has to say so
  log("step: a location with no trade");
  await press("t");
  // the other order from `load`'s: a coordinate is known in either
  await pick(1, "Clermont-Ferrand");
  await pick(0, "(no trade · grid only)");
  await take();
  for (let i = 0; i < 240; i++) {
    if ((await tabs()).length === 2) break;
    await sleep(1000);
  }
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand", "Clermont-Ferrand"], "a location on its own is named after the city");
  assert.equal(await layers(), 5, "the location's five INSEE layers, and not one that needs a demand model");
  // the city's own candidate is still a pin — a premises is the location's, not the trade's
  await expectMarkers(1, "nothing was bought, so no competitor is on the map");
  assert(await evaluate(`${ctl("Rank top 10 sites")}.disabled`), "the sweep has no demand to sweep");
  assert(await evaluate("document.querySelector('#ctl input[type=range]').disabled"), "and λ has no catchment to set");
  await press("w");
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand"], "`w` leaves the trade study behind it");

  log("step: escape");
  await press("t");
  await stroke("Escape", 27);
  assert(!(await pickerUp()), "Escape in a field closes the picker rather than being eaten by it");

  log("step: the picker by keyboard");
  await press("t");
  assert.deepEqual(
    await rows(0),
    ["car_detailing", "cleaning", "plumbing", "(no trade · grid only)"],
    "one field per axis, and this one is the trades — with the city-on-its-own row last",
  );
  assert.deepEqual(await rows(1), ["Angers", "Clermont-Ferrand", "Lyon"], "the locations are up at the same time, not after");

  // the two fields narrow independently — which is the whole of what a step sequence could not do
  await press("c");
  await press("l");
  await press("e");
  assert.deepEqual(await rows(0), ["cleaning"], "the caret starts in the trade field");
  assert.deepEqual(await rows(1), ["Angers", "Clermont-Ferrand", "Lyon"], "and narrowing the trade leaves the locations alone");
  await stroke("Enter", 13);
  assert.equal(await picked(0), "cleaning", "Enter picks the top row of the field it is in");
  assert(await pickerUp(), "and loads nothing");

  // half a pairing opens nothing, and the half that is missing says so
  await take();
  assert(await pickerUp(), "Load with no location leaves the picker up");
  assert.equal(await invalid(1), "true", "and marks the location field");
  assert.equal(await invalid(0), null, "and only that one");
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand"], "nothing was opened");

  // the pick is a value and not a row, so the query it was found through is free to move after
  await stroke("Backspace", 8);
  assert.deepEqual((await rows(0)).sort(), ["car_detailing", "cleaning"], "widening the query brings the other trades back");
  assert.equal(await picked(0), "cleaning", "and leaves the pick where it was made");
  await stroke("Tab", 9);
  assert.equal(await focused(), 1, "native Tab order crosses to the location field");
  await press("C");
  assert.deepEqual(await rows(1), ["Clermont-Ferrand"], "which narrows on its own");
  assert.deepEqual(await picked(1), null, "and has picked nothing yet");
  await stroke("Enter", 13);
  assert.equal(await picked(1), "Clermont-Ferrand", "Enter picks there too");
  assert.equal(await invalid(1), null, "and a pick clears the mark");
  assert(await pickerUp(), "a second pick still loads nothing");
  await stroke("Tab", 9);
  assert.equal(await focused(), "load", "and Tab carries on to Load");
  await stroke("Enter", 13);
  // the second pairing is evaluated on this request, which reads the grid archive again
  for (let i = 0; i < 240; i++) {
    if ((await tabs()).length === 2) break;
    await sleep(1000);
  }
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand", "cleaning_-_Clermont-Ferrand"], "the picked pairing is a second tab");
  assert.equal(await layers(), 12, "the cleaning study's eight layers are what the panel now offers");
  assert(
    (await evaluate("document.querySelector('#ctl div.note').innerText")).includes("10446 cells"),
    "the second study covers the same grid",
  );

  await press("[");
  assert.equal(await layers(), 10, "`[` goes back to the detailing study");
  await press("]");
  assert.equal(await layers(), 12, "`]` comes forward again");

  log("step: find an open tab");
  await press("[");
  await press("f");
  assert.deepEqual(await rows(0), ["car_detailing_-_Clermont-Ferrand", "cleaning_-_Clermont-Ferrand"], "`f` lists the open tabs");
  await pick(0, "cleaning_-_Clermont-Ferrand");
  assert(!(await pickerUp()), "a pick there is the whole motion");
  assert.equal(await layers(), 12, "and switches to the tab picked");

  await press("w");
  assert.deepEqual(await tabs(), ["car_detailing_-_Clermont-Ferrand"], "`w` closes the active tab");
  assert.equal(await layers(), 10, "and the panel is the surviving study's");
  await expectMarkers(131, "so is the map");

  assert.deepEqual(errors, [], "no uncaught exceptions");
  log("\nOK — all assertions passed");
  chrome.kill();
  process.exit(0);
}

main().catch(e => { console.error("FAIL:", e.message); chrome.kill(); process.exit(1); });
