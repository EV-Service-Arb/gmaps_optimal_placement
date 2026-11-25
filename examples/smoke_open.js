/* `view --open`, its `xdg-open` a headless Chromium. Run: node smoke_open.js <binary> view --port <p> .. */
const { spawn } = require("node:child_process");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const [BIN, ...ARGS] = process.argv.slice(2);
const PORT = ARGS[ARGS.indexOf("--port") + 1];
assert(BIN && ARGS.includes("--port"), "usage: node smoke_open.js <binary> view --port <p> ..");
const URL_ = `http://127.0.0.1:${PORT}`;
const CDP = "http://127.0.0.1:9334";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gop_smoke_open_"));
const opener = path.join(dir, "xdg-open");
fs.writeFileSync(opener, `#!/bin/sh
exec chromium --headless=new --disable-gpu --no-sandbox --user-data-dir=${dir}/profile --remote-debugging-port=9334 "$1" 2>/dev/null
`, { mode: 0o755 });

const sleep = ms => new Promise(r => setTimeout(r, ms));
const log = s => fs.writeSync(1, s + "\n");
const targets = async () => { try { return await (await fetch(`${CDP}/json/list`)).json(); } catch { return []; } };
const page = async () => (await targets()).find(t => t.type === "page" && t.url.startsWith(URL_));
const until = async (f, what, s) => {
  for (let i = 0; i < s * 10; i++) { const v = await f(); if (v) return v; await sleep(100); }
  throw new Error(`${what}: not within ${s} s`);
};
const call = async (url, method) => {
  const ws = new WebSocket(url);
  await new Promise((r, e) => { ws.onopen = r; ws.onerror = e; });
  await new Promise(r => { ws.onmessage = r; ws.onclose = r; ws.send(JSON.stringify({ id: 1, method })); });
  ws.close();
};
const quit = async () => {
  let v;
  try { v = await (await fetch(`${CDP}/json/version`)).json(); } catch { return; } // gone with its last page
  await call(v.webSocketDebuggerUrl, "Browser.close");
  await until(async () => { try { await fetch(`${CDP}/json/version`); return false; } catch { return true; } }, "the browser gone", 10);
};

let server = null;
const serve = async (...more) => {
  let ended = null;
  server = spawn(BIN, [...ARGS, ...more], { stdio: ["ignore", "inherit", "inherit"], env: { ...process.env, PATH: `${dir}:${process.env.PATH}` } });
  server.on("exit", (code, signal) => { ended = { code, signal }; });
  return { int: () => server.kill("SIGINT"), ended: () => ended };
};
const opened = async () => {
  const p = await until(async () => { const t = await page(); return t?.title === "studies" && t; }, "the page", 60);
  await sleep(2000); // its script has run
  return p;
};

async function main() {
  assert(!(await targets()).length, `${CDP} is another browser's`);

  let s = await serve("--open");
  let p = await opened();
  await call(p.webSocketDebuggerUrl, "Page.reload");
  await sleep(6000);
  assert.equal(s.ended(), null, "a reload is not a close");
  s.int();
  assert.deepEqual(await until(s.ended, "the interrupted server's end", 5), { code: 0, signal: null });
  assert(!(await page()), "an interrupted server closes its page");
  log("interrupted: the page is closed");
  await quit();

  s = await serve("-o");
  p = await opened();
  await fetch(`${CDP}/json/close/${p.id}`);
  assert.deepEqual(await until(s.ended, "the end of a server whose page is closed", 10), { code: 0, signal: null });
  log("closed: the server ended");
  await quit();

  s = await serve();
  await until(async () => { try { return (await fetch(URL_)).ok; } catch { return false; } }, "the server", 30);
  const browser = () => spawn(opener, [URL_], { stdio: "ignore" });
  browser();
  p = await opened();
  await fetch(`${CDP}/json/close/${p.id}`);
  await sleep(6000);
  assert.equal(s.ended(), null, "without --open a closed page ends nothing");
  await quit();
  browser();
  await opened();
  s.int();
  assert.deepEqual(await until(s.ended, "the interrupted server's end", 5), { code: null, signal: "SIGINT" });
  await sleep(2000);
  assert(await page(), "without --open an interrupt closes no page");
  log("without --open: neither");
  await quit();

  fs.rmSync(dir, { recursive: true });
  log("\nOK — all assertions passed");
}

main().then(() => process.exit(0), async e => { console.error("FAIL:", e.message); server?.kill("SIGKILL"); await quit(); process.exit(1); });
