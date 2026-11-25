/* Drives the GUI in headless Chromium over CDP. Run: node smoke.js <url>
 *
 * Only what is genuinely browser-shaped lives here: the island hydrates, the picker takes a pairing,
 * the overlay paints and its dots attach. The model's own numbers are `cargo t -p
 * gmaps_optimal_placement_core`'s.
 *
 * The pairing is one the work dir and the copy of prod's panel.db hold competitors for, picked by
 * the start of its labels: SMOKE_CATEGORY, SMOKE_LOCATION. A run of it is bought of a conductor played
 * here, on SMOKE_CONDUCTOR_PORT: what the server was given as `--conductor`. */
const { spawn } = require("node:child_process");
const assert = require("node:assert");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");

const URL_ = process.argv[2];
assert(URL_, "usage: node smoke.js <url>");
const CATEGORY = process.env.SMOKE_CATEGORY || "Plombier·FR";
const LOCATION = process.env.SMOKE_LOCATION || "lyon";
const PORT = 9333;

const conductor = { pulls: [], challenge: null };
http.createServer(async (req, res) => {
  const at = new URL(req.url, "http://conductor");
  let body = "";
  for await (const chunk of req) body += chunk;
  const answer = (status, v, headers = {}) => { res.writeHead(status, { "content-type": "application/json", ...headers }); res.end(JSON.stringify(v)); };
  const signedIn = req.headers.authorization === "Bearer smoke";
  switch (req.method + " " + at.pathname.replace(/[0-9a-f]{64}$/, "KEY")) {
    case "POST /mcp/register": return answer(201, { client_id: "smoke-client" });
    case "GET /mcp/authorize": {
      conductor.challenge = at.searchParams.get("code_challenge");
      const back = new URL(at.searchParams.get("redirect_uri"));
      back.search = new URLSearchParams({ code: "smoke-code", state: at.searchParams.get("state") });
      return answer(303, {}, { location: back.href });
    }
    case "POST /mcp/token": {
      const verifier = new URLSearchParams(body).get("code_verifier");
      const held = crypto.createHash("sha256").update(verifier).digest("base64url") === conductor.challenge;
      return held ? answer(200, { access_token: "smoke" }) : answer(400, { error: "invalid_grant" });
    }
    case "GET /api/v1/tokens": return signedIn ? answer(200, { balance: 100000, prices: { "placement.search": 8 } }) : answer(401, { error: "sign in" });
    case "POST /api/v1/placement/pull": {
      if (!signedIn) return answer(401, { error: "sign in" });
      conductor.pulls.push(JSON.parse(body));
      res.writeHead(200, { "content-type": "text/event-stream" });
      return res.end(":\n\nevent: pulled\ndata: {}\n\n");
    }
    case "GET /api/queries_cache/gop/KEY": {
      const { lat, lng } = conductor.pulls.at(-1).area.center;
      const places = [{ id: "smoke", displayName: { text: "Plomberie de la fumée" }, location: { latitude: lat, longitude: lng }, rating: 4.5, userRatingCount: 10 }];
      return answer(200, { answer: { places }, fetched_at: new Date().toISOString() });
    }
    default: return answer(404, { error: "no such route" });
  }
}).listen(Number(process.env.SMOKE_CONDUCTOR_PORT), "127.0.0.1");

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

  // the map's corner, which no competitor sits on
  await sleep(1000);
  const at = { x: 1300, y: 700 };
  const click = async (button, buttons) => {
    await send("Input.dispatchMouseEvent", { type: "mousePressed", ...at, button, buttons, clickCount: 1 });
    await send("Input.dispatchMouseEvent", { type: "mouseReleased", ...at, button, buttons: 0, clickCount: 1 });
  };
  // a dot eats what is under it, and the frame is wider than the study, so not every point answers
  let hint = null;
  for (let x = 500; x < 1200 && !hint; x += 45) {
    await send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y: 460 });
    await sleep(200);
    hint = await evaluate("(h => h.style.display === 'block' ? h.textContent : null)(document.querySelector('#map .gop-hint'))");
  }
  log("hint:", hint);
  assert(/pressure/.test(hint), "a cell under the cursor says what is known of it");
  await click("left", 1);
  await sleep(1000);
  assert(!(await evaluate("!!document.getElementById('report')")), "a left click on the map leaves nothing");
  await evaluate("window.addEventListener('contextmenu', e => { window.menu = e; }, true)");
  await click("right", 2);
  await until("!!document.getElementById('report')", "a card for the right click", 20);
  assert(await evaluate("window.menu.defaultPrevented"), "and the browser's own menu stays shut");
  log("card:", await evaluate("document.getElementById('report').innerText.slice(0, 120)"));

  // what is open is what the picker opens on
  const run = async () => {
    await send("Input.dispatchKeyEvent", { type: "keyDown", key: "t", text: "t" });
    await until("!!document.querySelector('#picker')", "the picker again", 20);
    await evaluate("[...document.querySelectorAll('#picker [data-slot=button]')].find(b => b.textContent === 'Run').click()");
    await until("!!document.querySelector('#confirm [role=alertdialog]')", "the confirmation", 120);
  };
  const decide = slot => evaluate("document.querySelector('#confirm [data-slot=" + slot + "]').click()");
  await run();
  const asked = await evaluate("document.querySelector('#confirm [role=alertdialog]').innerText");
  log("asked:", asked);
  assert(/at least \d+ searches/.test(asked) && /oldest \d+ d/.test(asked) && /100000/.test(asked), "what a run buys, what it is in place of, and of what balance");
  await decide("alert-dialog-cancel");
  await sleep(1000);
  assert.equal(conductor.pulls.length, 0, "a run not confirmed buys nothing");
  await run();
  await decide("alert-dialog-action");
  await until("/oldest 0 d/i.test(document.querySelector('#ctl').innerText)", "a map as of the run");
  assert.equal(conductor.pulls.length, 1, "a run is one pull");
  assert.equal(conductor.pulls[0].category.trade.name, CATEGORY.split("·")[0], "of the pairing picked");
  assert.equal(await evaluate("document.querySelectorAll('#tabs .tab:not(.add)').length"), 1, "in the tab it was open in");
  assert.equal(await evaluate("document.querySelector('#banner')?.innerText ?? null"), null, "and whole");

  const forged = await fetch(new URL("run/" + encodeURIComponent(CATEGORY) + "/" + LOCATION, URL_), { method: "POST", headers: { origin: "http://evil.example" } });
  assert.equal(forged.status, 403, "another site's page runs nothing");

  assert.deepEqual(errors, [], "no uncaught exceptions");
  log("\nOK — all assertions passed");
  chrome.kill();
  process.exit(0);
}

main().catch(e => { console.error("FAIL:", e.message); chrome.kill(); process.exit(1); });
