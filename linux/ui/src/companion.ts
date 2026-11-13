/* AirBridge phone companion: share with the computer from any phone browser. */
"use strict";

const { icons, fileIcon, sharePhrase, bytes, esc, el, toast } = AB;
/** An element the page's HTML always has. */
const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

/** The computer this page is paired with (`/hello`, or the socket's hello). */
interface Desktop {
  alias: string;
  deviceModel?: string;
  /** Android serves no WebSocket: poll instead. */
  poll?: boolean;
  /** false when the computer cannot take text messages. */
  text?: boolean;
}

interface OfferFile extends NamedFile {
  id: string;
  size: number;
}

/** Something the computer wants to send to this phone. */
interface Offer {
  id: string;
  from: string;
  message?: string;
  files?: OfferFile[];
}

interface Recent {
  icon: string;
  title: string;
  sub: string;
  copy?: string;
  href?: string;
}

const store = {
  get(key: string) { try { return localStorage.getItem(key); } catch (_) { return null; } },
  set(key: string, value: string) { try { localStorage.setItem(key, value); } catch (_) { /* private mode */ } },
};

// The pairing key arrives in the URL fragment (never sent over the network as part of the URL path).
const hashKey = new URLSearchParams(location.hash.slice(1)).get("k");
if (hashKey) {
  store.set("ab_key", hashKey);
  history.replaceState(null, "", location.pathname + location.search);
}
const key = hashKey || store.get("ab_key");
let clientId = store.get("ab_id");
if (!clientId) {
  clientId = [...crypto.getRandomValues(new Uint8Array(10))].map((b) => b.toString(16).padStart(2, "0")).join("");
  store.set("ab_id", clientId);
}

const session: { desktop: Desktop | null; ws: WebSocket | null; busy: boolean; offer: Offer | null; recent: Recent[] } =
  { desktop: null, ws: null, busy: false, offer: null, recent: [] };

function guessName() {
  const ua = navigator.userAgent;
  const android = ua.match(/Android [\d.]+; ([^;)]+?)(?: Build|\))/);
  if (android && android[1] && android[1] !== "K") return android[1].trim();
  if (/iPhone/.test(ua)) return "iPhone";
  if (/iPad/.test(ua)) return "iPad";
  if (/Android/.test(ua)) return "Android phone";
  return "Browser";
}
let deviceName = store.get("ab_name") || guessName();
let deviceModel = "";

async function detectModel() {
  try {
    // User-Agent Client Hints: Chromium only, not in TypeScript's DOM types yet.
    const hints = (navigator as Navigator & { userAgentData?: { getHighEntropyValues(keys: string[]): Promise<{ model?: string }> } }).userAgentData;
    const data = await hints?.getHighEntropyValues(["model"]);
    if (data && data.model) {
      deviceModel = data.model;
      if (!store.get("ab_name")) deviceName = data.model;
    }
  } catch (_) { /* not supported */ }
}

const query = (extra: Record<string, string> = {}) => new URLSearchParams({ k: key || "", id: clientId || "", ...extra }).toString();

/* ------------------------------------------------------------ connection */
function setLink(connected: boolean, text: string) {
  $("linkDot").classList.toggle("off", !connected);
  $("linkText").textContent = text;
}

async function checkPairing(): Promise<boolean> {
  if (!key) return false;
  try {
    const res = await fetch(`/hello?${query()}`);
    if (res.status === 403) return false;
    const data = await res.json();
    session.desktop = data;
    return true;
  } catch (_) {
    return true; // offline for a moment; keep trying
  }
}

let retry = 0;
function connect(): void {
  if (session.desktop?.poll) {
    setLink(true, `Connected to ${session.desktop.alias}`);
    $("targetName").textContent = session.desktop.alias;
    if (session.desktop.text === false) $("btnText").hidden = true;
    poll();
    return;
  }
  const ws = new WebSocket(`ws://${location.host}/ws?${query({ name: deviceName, model: deviceModel })}`);
  session.ws = ws;
  ws.onopen = () => { retry = 0; };
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.type === "hello") {
      session.desktop = { alias: msg.alias, deviceModel: msg.deviceModel };
      setLink(true, `Connected to ${msg.alias}`);
      $("targetName").textContent = msg.alias;
    } else if (msg.type === "offer") {
      showOffer(msg);
    } else if (msg.type === "cancel") {
      if (session.offer && session.offer.id === msg.id) closeOffer();
      toast("Cancelled", "The computer cancelled the transfer");
    }
  };
  ws.onclose = async () => {
    setLink(false, "Reconnecting…");
    if (!(await checkPairing())) return showUnpaired();
    retry = Math.min(retry + 1, 6);
    setTimeout(connect, 600 * retry);
  };
}

// Android's small HTTP server uses polling; Linux keeps its WebSocket transport.
let lastOffer: string | null = null;
async function poll(): Promise<void> {
  try {
    const res = await fetch(`/poll?${query()}`);
    if (res.status === 403 || res.status === 410) return showUnpaired();
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const alias = session.desktop?.alias || "your computer";
    setLink(true, `Connected to ${alias}`);
    $("targetName").textContent = alias;
    for (const msg of (await res.json()) as (Offer & { type: string })[]) {
      if (msg.type === "cancel" && session.offer?.id === msg.id) {
        closeOffer();
        toast("Cancelled", "The sender cancelled the transfer");
      }
      if (msg.type === "offer" && msg.id !== lastOffer) {
        lastOffer = msg.id;
        showOffer(msg);
      }
    }
    setTimeout(poll, 500);
  } catch (_) {
    setLink(false, "Reconnecting…");
    setTimeout(poll, 2000);
  }
}

function showUnpaired() {
  $("home").hidden = true;
  $("unpaired").hidden = false;
  setLink(false, "Not paired");
}

/* --------------------------------------------------------- share target */
/** The computer's ring and status line, like a device in the iOS share sheet. */
function setTarget(mode: "idle" | "busy" | "success", progress = 0, status?: string) {
  const home = $("home");
  home.classList.toggle("busy", mode === "busy");
  home.classList.toggle("success", mode === "success");
  $("targetBar").style.strokeDashoffset = String(mode === "success" ? 0 : 289 * (1 - progress));
  $("targetStatus").textContent = status || "Tap to send files";
}

function addRecent(item: Recent) {
  session.recent.unshift(item);
  session.recent = session.recent.slice(0, 12);
  renderRecent();
}

function renderRecent() {
  const box = $("recent");
  if (!session.recent.length) {
    box.innerHTML = "";
    return;
  }
  box.innerHTML = `<p class="recent-title">This session</p>`;
  const list = el(`<div class="list"></div>`);
  for (const r of session.recent) {
    const node = el(`<div class="row">
      <span class="r-icon">${r.icon}</span>
      <span class="r-text">${esc(r.title)}<small>${esc(r.sub)}</small></span>
      ${r.copy ? `<button class="r-act" data-copy>Copy</button>` : ""}
      ${r.href ? `<a class="r-act" href="${r.href}" download>Save</a>` : ""}
    </div>`);
    const copy = r.copy;
    if (copy) node.querySelector("[data-copy]")!.addEventListener("click", async (e) => {
      (e.currentTarget as HTMLElement).textContent = (await AB.copyText(copy)) ? "Copied" : "Copy failed";
    });
    list.appendChild(node);
  }
  box.appendChild(list);
}

/* ------------------------------------------------------------- sending */
async function sendFiles(files: File[]) {
  if (!files.length || session.busy) return;
  session.busy = true;
  const total = files.reduce((s, f) => s + f.size, 0);
  setTarget("busy", 0, "Waiting…");
  try {
    const res = await fetch(`/send?${query()}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ files: files.map((f) => ({ name: f.name, size: f.size, type: f.type })) }),
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.message || "The computer refused");
    const loaded = files.map(() => 0);
    const started = performance.now();
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(3, files.length) }, async () => {
      while (next < files.length) {
        const i = next++;
        await upload(files[i], data.transfer, data.files[i].id, (n: number) => {
          loaded[i] = n;
          const done = loaded.reduce((sum, value) => sum + value, 0);
          const secs = (performance.now() - started) / 1000;
          setTarget("busy", total ? done / total : 1, `Sending… ${bytes(done / Math.max(secs, 0.1))}/s`);
        });
      }
    }));
    setTarget("success", 1, "Sent");
    if (navigator.vibrate) navigator.vibrate(18);
    addRecent({
      icon: files.length === 1 ? fileIcon(files[0].type, files[0].name) : icons.folder,
      title: files.length === 1 ? files[0].name : `${files.length} files`,
      sub: `Sent · ${bytes(total)}`,
    });
    setTimeout(() => setTarget("idle"), 2600);
  } catch (err) {
    toast("Couldn't send", errorText(err), "bad");
    setTarget("idle");
  } finally {
    session.busy = false;
  }
}

function upload(file: File, transfer: string, fileId: string, onProgress: (loaded: number) => void): Promise<void> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", `/upload?${query({ transfer, file: fileId })}`);
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
    xhr.onload = () => (xhr.status === 200 ? resolve() : reject(new Error(xhr.responseText || `HTTP ${xhr.status}`)));
    xhr.onerror = () => reject(new Error("Connection lost"));
    xhr.send(file);
  });
}

function composeText() {
  const modal = $("modal");
  modal.innerHTML = `<div class="modal-card"><h2>Send a message</h2><p class="muted">Links, codes, notes — it lands on your computer instantly.</p>
    <textarea class="input" id="msg" placeholder="Type or paste…"></textarea>
    <div class="actions"><button class="btn ghost" id="msgCancel">Cancel</button><button class="btn" id="msgSend">${icons.send} Send</button></div></div>`;
  modal.hidden = false;
  const close = () => { modal.hidden = true; modal.innerHTML = ""; };
  $("msg").focus();
  $("msgCancel").onclick = close;
  modal.onclick = (e) => { if (e.target === modal) close(); };
  $("msgSend").onclick = async () => {
    const text = $<HTMLTextAreaElement>("msg").value;
    if (!text.trim()) return;
    close();
    try {
      const res = await fetch(`/text?${query()}`, { method: "POST", headers: { "content-type": "text/plain;charset=utf-8" }, body: text });
      if (!res.ok) throw new Error("The computer refused");
      toast("Message sent", text.length > 60 ? text.slice(0, 60) + "…" : text, "ok");
      addRecent({ icon: icons.message, title: text.split("\n")[0].slice(0, 60), sub: "Message sent" });
      if (navigator.vibrate) navigator.vibrate(15);
    } catch (err) {
      toast("Couldn't send", errorText(err), "bad");
    }
  };
}

function rename() {
  const modal = $("modal");
  modal.innerHTML = `<div class="modal-card"><h2>Your name</h2><p class="muted">How this phone appears on the computer.</p>
    <input class="input" id="nameInput" maxlength="40" value="${esc(deviceName)}">
    <div class="actions"><button class="btn ghost" id="nameCancel">Cancel</button><button class="btn" id="nameSave">Save</button></div></div>`;
  modal.hidden = false;
  const close = () => { modal.hidden = true; modal.innerHTML = ""; };
  $("nameCancel").onclick = close;
  $("nameSave").onclick = () => {
    deviceName = $<HTMLInputElement>("nameInput").value.trim() || guessName();
    store.set("ab_name", deviceName);
    $("youName").textContent = deviceName;
    close();
    session.ws && session.ws.close(); // reconnect with the new name
  };
}

/* -------------------------------------------------------------- receiving */
function showOffer(offer: Offer) {
  session.offer = offer;
  const files = (offer.files || []).filter((f) => !offer.message || f.name !== `${f.id}.txt`);
  const isText = offer.message && files.length === 0;
  const first: Partial<OfferFile> = files[0] || {};
  $("offerPreview").innerHTML = isText ? `<div class="bubble">${esc(offer.message)}</div>`
    : `<div class="tile">${files.length > 1 && !files.every((f) => f.type === first.type) ? icons.file : fileIcon(first.type, first.name)}</div>`;
  $("offerText").textContent = `“${offer.from}” would like to share ${isText ? "a message" : sharePhrase(files)}.`;
  $("offerMeta").textContent = isText ? "" : bytes(files.reduce((sum, f) => sum + (f.size || 0), 0));
  $("offerAccept").textContent = isText ? "Copy" : "Accept";
  $("offerDecline").textContent = isText ? "Close" : "Decline";
  $("sheetBackdrop").classList.add("open");
  $("offerSheet").classList.add("open");
  if (navigator.vibrate) navigator.vibrate(25);
}

function closeOffer() {
  session.offer = null;
  $("sheetBackdrop").classList.remove("open");
  $("offerSheet").classList.remove("open");
}

async function answerOffer(accept: boolean) {
  const offer = session.offer;
  if (!offer) return;
  closeOffer();
  const files = (offer.files || []).filter((f) => !offer.message || f.name !== `${f.id}.txt`);
  const isText = offer.message && files.length === 0;
  const message = offer.message || "";
  if (isText && accept) await AB.copyText(message);
  await fetch(`/offer/${offer.id}/${accept ? "accept" : "decline"}?${query()}`, { method: "POST" }).catch(() => {});
  if (!accept) return;
  if (isText) {
    toast("Copied", "", "ok", 1600);
    addRecent({ icon: icons.message, title: message.split("\n")[0].slice(0, 60), sub: `From ${offer.from}`, copy: message });
    return;
  }
  // Native downloads: one after another so the browser keeps up.
  for (const f of files) {
    const href = `/download/${offer.id}/${encodeURIComponent(f.id)}?${query()}`;
    const a = document.createElement("a");
    a.href = href;
    a.download = f.name.split("/").pop() || f.name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    addRecent({ icon: fileIcon(f.type, f.name), title: f.name.split("/").pop() || f.name, sub: `From ${offer.from} · ${bytes(f.size)}`, href });
    await new Promise((r) => setTimeout(r, 700));
  }
  toast("Saving to Downloads", `${files.length} file${files.length > 1 ? "s" : ""} from ${offer.from}`, "ok");
}

/* ------------------------------------------------------------------ init */
async function init() {
  $("unpairedIcon").innerHTML = icons.qr;
  $("targetIcon").innerHTML = icons.laptop;
  $("btnPhotos").querySelector(".r-icon")!.innerHTML = icons.image;
  $("btnFiles").querySelector(".r-icon")!.innerHTML = icons.folder;
  $("btnText").querySelector(".r-icon")!.innerHTML = icons.message;
  document.querySelectorAll(".chev").forEach((c) => { c.innerHTML = icons.chevron; });
  setTarget("idle");

  $("target").addEventListener("click", () => $("pickAny").click());
  $("btnFiles").addEventListener("click", () => $("pickAny").click());
  $("btnPhotos").addEventListener("click", () => $("pickMedia").click());
  $("btnText").addEventListener("click", composeText);
  $("youChip").addEventListener("click", rename);
  for (const id of ["pickAny", "pickMedia"]) {
    const input = $<HTMLInputElement>(id);
    input.addEventListener("change", () => {
      sendFiles([...(input.files || [])]);
      input.value = "";
    });
  }
  $("offerAccept").addEventListener("click", () => answerOffer(true));
  $("offerDecline").addEventListener("click", () => answerOffer(false));
  addEventListener("dragover", (e) => { e.preventDefault(); $("home").classList.add("drag"); });
  addEventListener("dragleave", () => $("home").classList.remove("drag"));
  addEventListener("drop", (e) => {
    e.preventDefault();
    $("home").classList.remove("drag");
    sendFiles([...(e.dataTransfer?.files || [])]);
  });

  if (!(await checkPairing())) return showUnpaired();
  await detectModel();
  $("youName").textContent = deviceName;
  $("home").hidden = false;
  if (session.desktop) {
    setLink(false, `Connecting to ${session.desktop.alias}…`);
    $("targetName").textContent = session.desktop.alias;
  }
  connect();
}

init();
