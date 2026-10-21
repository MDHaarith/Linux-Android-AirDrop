/* AirBridge phone companion: share with the computer from any phone browser. */
"use strict";

const { icons, fileIcon, paintAvatar, bytes, esc, el, toast, fx, center } = AB;
const $ = (id) => document.getElementById(id);

const store = {
  get(key) { try { return localStorage.getItem(key); } catch (_) { return null; } },
  set(key, value) { try { localStorage.setItem(key, value); } catch (_) { /* private mode */ } },
};

// The pairing key arrives in the URL fragment (never sent over the network as part of the URL path).
const hashKey = new URLSearchParams(location.hash.slice(1)).get("k");
if (hashKey) {
  store.set("ab_key", hashKey);
  history.replaceState(null, "", location.pathname);
}
const key = hashKey || store.get("ab_key");
let clientId = store.get("ab_id");
if (!clientId) {
  clientId = [...crypto.getRandomValues(new Uint8Array(10))].map((b) => b.toString(16).padStart(2, "0")).join("");
  store.set("ab_id", clientId);
}

const session = { desktop: null, ws: null, busy: false, offer: null, recent: [] };

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
    const data = await navigator.userAgentData?.getHighEntropyValues(["model"]);
    if (data && data.model) {
      deviceModel = data.model;
      if (!store.get("ab_name")) deviceName = data.model;
    }
  } catch (_) { /* not supported */ }
}

const query = (extra = {}) => new URLSearchParams({ k: key, id: clientId, ...extra }).toString();

/* ------------------------------------------------------------ connection */
function setLink(connected, text) {
  $("linkDot").classList.toggle("off", !connected);
  $("linkText").textContent = text;
}

async function checkPairing() {
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
function connect() {
  const ws = new WebSocket(`ws://${location.host}/ws?${query({ name: deviceName, model: deviceModel })}`);
  session.ws = ws;
  ws.onopen = () => { retry = 0; };
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.type === "hello") {
      session.desktop = { alias: msg.alias, deviceModel: msg.deviceModel };
      setLink(true, `Connected to ${msg.alias}`);
      $("orbSub").textContent = `to ${msg.alias}`;
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

function showUnpaired() {
  $("home").hidden = true;
  $("unpaired").hidden = false;
  setLink(false, "Not paired");
}

/* ---------------------------------------------------------------- orb ui */
function setOrb(mode, progress = 0, label, sub) {
  const home = $("home");
  home.classList.toggle("busy", mode === "busy");
  home.classList.toggle("success", mode === "success");
  $("orbBar").style.strokeDashoffset = 578 * (1 - progress);
  $("orbIcon").innerHTML = mode === "success" ? icons.check : mode === "busy" ? icons.upload : icons.send;
  $("orbLabel").textContent = label || "Send files";
  $("orbSub").textContent = sub || (session.desktop ? `to ${session.desktop.alias}` : "to your computer");
}

function addRecent(item) {
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
  box.innerHTML = `<h3>This session</h3>`;
  session.recent.forEach((r, i) => {
    const node = el(`<div class="r-item" style="animation-delay:${i * 40}ms">
      <div class="r-icon">${r.icon}</div>
      <div class="r-body"><div class="r-title">${esc(r.title)}</div><div class="r-sub">${esc(r.sub)}</div></div>
      ${r.copy ? `<button class="link" data-copy>Copy</button>` : ""}
      ${r.href ? `<a href="${r.href}" download>Save</a>` : ""}
    </div>`);
    if (r.copy) node.querySelector("[data-copy]").addEventListener("click", async (e) => {
      e.target.textContent = (await AB.copyText(r.copy)) ? "Copied ✓" : "Copy failed";
    });
    box.appendChild(node);
  });
}

/* ------------------------------------------------------------- sending */
async function sendFiles(files) {
  if (!files.length || session.busy) return;
  session.busy = true;
  const total = files.reduce((s, f) => s + f.size, 0);
  setOrb("busy", 0, "Preparing…", `${files.length} item${files.length > 1 ? "s" : ""} · ${bytes(total)}`);
  try {
    const res = await fetch(`/send?${query()}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ files: files.map((f) => ({ name: f.name, size: f.size, type: f.type })) }),
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.message || "The computer refused");
    let sentBefore = 0;
    const started = performance.now();
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      await upload(file, data.transfer, data.files[i].id, (loaded) => {
        const done = sentBefore + loaded;
        const secs = (performance.now() - started) / 1000;
        setOrb("busy", total ? done / total : 1, `${Math.round((total ? done / total : 1) * 100)}%`, `${bytes(done / Math.max(secs, 0.1))}/s · ${file.name}`);
      });
      sentBefore += file.size;
    }
    setOrb("success", 1, "Sent!", `to ${session.desktop?.alias || "your computer"}`);
    const c = center($("orb"));
    if (c) fx.confetti(c.x, c.y, 120);
    if (navigator.vibrate) navigator.vibrate([18, 40, 18]);
    addRecent({
      icon: files.length === 1 ? fileIcon(files[0].type, files[0].name) : icons.folder,
      title: files.length === 1 ? files[0].name : `${files.length} files`,
      sub: `Sent · ${bytes(total)}`,
    });
    setTimeout(() => setOrb("idle"), 2600);
  } catch (err) {
    toast("Couldn't send", err.message, "bad");
    setOrb("idle");
  } finally {
    session.busy = false;
  }
}

function upload(file, transfer, fileId, onProgress) {
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
    const text = $("msg").value;
    if (!text.trim()) return;
    close();
    try {
      const res = await fetch(`/text?${query()}`, { method: "POST", headers: { "content-type": "text/plain;charset=utf-8" }, body: text });
      if (!res.ok) throw new Error("The computer refused");
      toast("Message sent", text.length > 60 ? text.slice(0, 60) + "…" : text, "ok");
      addRecent({ icon: icons.message, title: text.split("\n")[0].slice(0, 60), sub: "Message sent" });
      if (navigator.vibrate) navigator.vibrate(15);
    } catch (err) {
      toast("Couldn't send", err.message, "bad");
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
    deviceName = $("nameInput").value.trim() || guessName();
    store.set("ab_name", deviceName);
    $("youName").textContent = deviceName;
    close();
    session.ws && session.ws.close(); // reconnect with the new name
  };
}

/* -------------------------------------------------------------- receiving */
function showOffer(offer) {
  session.offer = offer;
  const files = (offer.files || []).filter((f) => !offer.message || f.name !== `${f.id}.txt`);
  const isText = offer.message && files.length === 0;
  $("offerTitle").textContent = isText ? `${offer.from} sent you a message` : `${offer.from} wants to send you ${files.length === 1 ? "a file" : files.length + " files"}`;
  const avatar = $("offerAvatar");
  avatar.innerHTML = icons.laptop;
  paintAvatar(avatar, offer.from || "");
  $("offerBody").innerHTML = isText
    ? `<div class="o-text">${esc(offer.message)}</div>`
    : files.map((f, i) => `<div class="o-file" style="animation-delay:${Math.min(i, 8) * 40}ms">${fileIcon(f.type, f.name)}<span>${esc(f.name.split("/").pop())}</span><small>${bytes(f.size)}</small></div>`).join("");
  $("offerAccept").textContent = isText ? "Copy" : "Accept";
  $("offerDecline").textContent = isText ? "Close" : "Decline";
  $("sheetBackdrop").classList.add("open");
  $("offerSheet").classList.add("open");
  if (navigator.vibrate) navigator.vibrate([25, 60, 25]);
}

function closeOffer() {
  session.offer = null;
  $("sheetBackdrop").classList.remove("open");
  $("offerSheet").classList.remove("open");
}

async function answerOffer(accept) {
  const offer = session.offer;
  if (!offer) return;
  closeOffer();
  const files = (offer.files || []).filter((f) => !offer.message || f.name !== `${f.id}.txt`);
  const isText = offer.message && files.length === 0;
  if (isText && accept) await AB.copyText(offer.message);
  await fetch(`/offer/${offer.id}/${accept ? "accept" : "decline"}?${query()}`, { method: "POST" }).catch(() => {});
  if (!accept) return;
  if (isText) {
    toast("Copied", "", "ok", 1600);
    addRecent({ icon: icons.message, title: offer.message.split("\n")[0].slice(0, 60), sub: `From ${offer.from}`, copy: offer.message });
    return;
  }
  // Native downloads: one after another so the browser keeps up.
  for (const f of files) {
    const href = `/download/${offer.id}/${encodeURIComponent(f.id)}?${query()}`;
    const a = document.createElement("a");
    a.href = href;
    a.download = f.name.split("/").pop();
    document.body.appendChild(a);
    a.click();
    a.remove();
    addRecent({ icon: fileIcon(f.type, f.name), title: f.name.split("/").pop(), sub: `From ${offer.from} · ${bytes(f.size)}`, href });
    await new Promise((r) => setTimeout(r, 700));
  }
  const c = center($("orb"));
  if (c) fx.confetti(c.x, c.y, 90);
  toast("Saving to Downloads", `${files.length} file${files.length > 1 ? "s" : ""} from ${offer.from}`, "ok");
}

/* ------------------------------------------------------------------ init */
async function init() {
  document.body.insertAdjacentHTML("afterbegin", `<svg width="0" height="0" style="position:absolute"><defs>
    <linearGradient id="orbGradient" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#8f7dff"/><stop offset="0.55" stop-color="#ff5fa2"/><stop offset="1" stop-color="#33d1ff"/></linearGradient></defs></svg>`);
  $("unpairedIcon").innerHTML = icons.qr;
  $("btnPhotos").innerHTML = `${icons.image} Photos`;
  $("btnText").innerHTML = `${icons.message} Message`;
  fx.init($("fx"));
  setOrb("idle");

  $("orb").addEventListener("click", () => $("pickAny").click());
  $("btnPhotos").addEventListener("click", () => $("pickMedia").click());
  $("btnText").addEventListener("click", composeText);
  $("youChip").addEventListener("click", rename);
  for (const id of ["pickAny", "pickMedia"]) {
    $(id).addEventListener("change", (e) => {
      sendFiles([...e.target.files]);
      e.target.value = "";
    });
  }
  $("offerAccept").addEventListener("click", () => answerOffer(true));
  $("offerDecline").addEventListener("click", () => answerOffer(false));
  addEventListener("dragover", (e) => { e.preventDefault(); $("home").classList.add("drag"); });
  addEventListener("dragleave", () => $("home").classList.remove("drag"));
  addEventListener("drop", (e) => {
    e.preventDefault();
    $("home").classList.remove("drag");
    sendFiles([...e.dataTransfer.files]);
  });

  if (!(await checkPairing())) return showUnpaired();
  await detectModel();
  $("youName").textContent = deviceName;
  $("home").hidden = false;
  if (session.desktop) setLink(false, `Connecting to ${session.desktop.alias}…`);
  connect();
}

init();
