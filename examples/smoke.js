/* Drives the GUI in headless Chromium over CDP. Run: node smoke.js <url>
 *
 * Only what is genuinely browser-shaped lives here: the island hydrates, the picker takes a pairing,
 * the overlay paints and its dots attach. The model's own numbers are `cargo t -p
 * gmaps_optimal_placement_core`'s.
 *
 * The pairing is one the work dir and the copy of prod's panel.db hold competitors for, picked by
 * the start of its labels: SMOKE_CATEGORY, SMOKE_LOCATION. */
const { spawn } = require("node:child_process");
const assert = require("node:assert");
const fs = require("node:fs");

const URL_ = process.argv[2];
assert(URL_, "usage: node smoke.js <url>");
const CATEGORY = process.env.SMOKE_CATEGORY || "Plombier·FR";
const LOCATION = process.env.SMOKE_LOCATION || "lyon";
const PORT = 9333;

const chrome = spawn("chromium", [
  "--headless=new", "--disable-gpu", "--no-sandbox", "--window-size=1400,900",
  `--remote-debugging-port=${PORT}`, "about:blank",
], { stdio: "ignore" });

const sleep = ms => new Promise(r => setTimeout(r, ms));
// unbuffered: a hang mid-run should still show how far it got
const log = (...a) => fs.writeSync(1, a.map(x => (typeof x === "string" ? x : JSON.stringify(x))).join(" ") + "\n");

async function main() {
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
  ws.addEventListener("message", ev => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); }
    if (m.method === "Runtime.exceptionThrown")
      errors.push(m.params.exceptionDetails.exception?.description || "exception");
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
  const until = async (expr, what, tries = 240) => {
    for (let i = 0; i < tries; i++) {
      if (await evaluate(expr).catch(() => false)) return;
      await sleep(500);
    }
    throw new Error(`${what} never happened; banner: ${await evaluate("document.querySelector('#banner')?.innerText ?? null")}`);
  };
  // one uikit `Command` per axis
  const items = n => `[...document.querySelectorAll('#picker [data-slot=command]')[${n}].querySelectorAll('[data-slot=command-item]')]`;
  const pick = (n, start) => evaluate(`(() => { const i = ${items(n)}.find(i => i.textContent.startsWith(${JSON.stringify(start)}));
    if (!i) throw new Error('no row starts with ' + ${JSON.stringify(start)}); i.click(); return i.textContent; })()`);

  await send("Runtime.enable");
  await send("Page.enable");
  await send("Page.navigate", { url: URL_ });
  await until("!!document.querySelector('#picker [data-slot=command-input]')", "the picker");
  log("picked:", await pick(0, CATEGORY), "×", await pick(1, LOCATION));
  await evaluate("document.querySelector('#picker [data-slot=button]').click()");
  await until("document.querySelectorAll('#ctl select option').length > 0 && document.querySelectorAll('#ticks span').length > 0", "the panel and its legend");

  const s = await evaluate(`(() => {
    const cv = document.querySelector('#map canvas');
    return {
      tabs: [...document.querySelectorAll('#tabs .tab:not(.add)')].map(t => t.textContent.replace('●', '')),
      layers: document.querySelectorAll('#ctl select option').length,
      note: document.querySelector('#ctl div.note')?.innerText.replace(/\\n/g, ' '),
      banner: document.querySelector('#banner')?.innerText ?? null,
      canvas: cv && [cv.width, cv.height],
      painted: cv ? (() => { const d = cv.getContext('2d').getImageData(0, 0, cv.width, cv.height).data;
        let n = 0; for (let i = 3; i < d.length; i += 4 * 97) if (d[i] > 0) n++; return n; })() : 0,
      dots: document.querySelectorAll('#map [role=button]').length,
    };
  })()`);
  log("state:", s);
  assert.equal(s.tabs.length, 1, "the pairing is the one open tab");
  assert(/\d+ cells in view/.test(s.note), `the legend counts the cells in view: ${s.note}`);
  assert(s.painted > 100, "the overlay's canvas actually painted");
  assert(s.dots > 0, "the competitors are on the map");

  // a click drops a pin and opens its card; the map's corner, which no competitor sits on
  await sleep(1000);
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: 1300, y: 700, button: "left", buttons: 1, clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: 1300, y: 700, button: "left", buttons: 0, clickCount: 1 });
  await until("!!document.getElementById('report')", "a card for the click", 20);
  log("card:", await evaluate("document.getElementById('report').innerText.slice(0, 120)"));

  assert.deepEqual(errors, [], "no uncaught exceptions");
  log("\nOK — all assertions passed");
  chrome.kill();
  process.exit(0);
}

main().catch(e => { console.error("FAIL:", e.message); chrome.kill(); process.exit(1); });
