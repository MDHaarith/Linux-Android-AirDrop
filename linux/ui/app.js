/* AirBridge desktop window. Talks to the local service over /api (cookie auth). */
"use strict";

const { icons, deviceIcon, fileIcon, sharePhrase, bytes, ago, esc, el, toast, center } = AB;
const $ = (id) => document.getElementById(id);

const state = {
  me: null,
  peers: new Map(),
  transfers: new Map(),
  settings: {},
  companion: {},
  multicast: true,
  staged: [],          // {key, kind: "path"|"upload", name, size, type, path?, id?, progress, done}
  pins: {},            // peer id -> PIN to use
  unseen: 0,
};
const peerEls = new Map();
const itemEls = new Map();
const lastStatus = new Map();
const incomingQueue = [];
let currentIncoming = null;
let hoverPeer = null;
let dragDepth = 0;
const ALL_DEVICES = "@all";
let dropTarget = null;

function recipients(target) {
  if (target === ALL_DEVICES) return [...state.peers.keys()];
  return state.peers.has(target) ? [target] : [];
}

function selectDropTarget(target) {
  dropTarget = target;
  api("/settings", { body: { dropRecipient: target } }).catch((err) => toast("Couldn't save default", err.message, "bad"));
  renderDropTarget();
}

function renderDropTarget() {
  for (const [id, node] of peerEls) {
    node.classList.toggle("selected", id === dropTarget);
    node.setAttribute("aria-pressed", String(id === dropTarget));
  }
  $("allPeers").hidden = state.peers.size === 0;
  $("allPeers").classList.toggle("selected", dropTarget === ALL_DEVICES);
  $("allPeers").setAttribute("aria-pressed", String(dropTarget === ALL_DEVICES));
  $("clearDropTarget").hidden = !dropTarget;
  $("dropPrompt").hidden = state.peers.size === 0;
  const name = dropTarget === ALL_DEVICES ? "all nearby devices" : state.peers.get(dropTarget)?.alias;
  $("dropRecipient").textContent = name ? `Drops here send to ${name}.` : dropTarget ? "Your default device is offline. Drop on another bubble to send." : "Choose a device on the left, or drop directly on its bubble.";
}

/* ------------------------------------------------------------------ api */
async function api(path, { method, body } = {}) {
  const res = await fetch("/api" + path, {
    method: method || (body !== undefined ? "POST" : "GET"),
    headers: body !== undefined ? { "content-type": "application/json" } : {},
    body: body !== undefined ? JSON.stringify(body) : undefined,
    credentials: "same-origin",
  });
  let data = null;
  try { data = await res.json(); } catch (_) { /* empty body */ }
  if (!res.ok) throw new Error((data && data.message) || `Request failed (${res.status})`);
  return data;
}

/* ---------------------------------------------------------------- updates */
async function checkUpdate() {
  try {
    const { available, canInstall } = await api("/update");
    const btn = $("btnUpdate");
    btn.hidden = !available;
    if (available) btn.title = `${available.tag} is ready` + (canInstall ? "" : " (opens the download page)");
  } catch (_) { /* offline: try again later */ }
}

async function installUpdate() {
  const btn = $("btnUpdate");
  btn.disabled = true;
  btn.textContent = "Updating…";
  try {
    const { restarting } = await api("/update", { body: {} });
    if (restarting) {
      toast("Updated. Restarting AirBridge…");
      // The new version opens its own window.
      setTimeout(() => window.ipc?.postMessage("close"), 300);
      return;
    }
  } catch (err) {
    toast(err.message);
  }
  btn.disabled = false;
  btn.textContent = "Update";
}

function applyTheme(theme) {
  if (!theme || theme === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}

/* ------------------------------------------------------------ live events */
function connect() {
  const ws = new WebSocket(`ws://${location.host}/api/events`);
  ws.onmessage = (e) => handle(JSON.parse(e.data));
  ws.onopen = () => setOnline(true);
  ws.onclose = () => {
    setOnline(false);
    setTimeout(connect, 1500);
  };
}

function setOnline(on) {
  $("liveDot").classList.toggle("off", !on);
  document.body.classList.toggle("offline", !on);
}

function handle({ event, data }) {
  switch (event) {
    case "state": return loadState(data);
    case "peer": return upsertPeer(data);
    case "peer-removed": return removePeer(data.id);
    case "transfer": return updateTransfer(data);
    case "incoming": return queueIncoming(data.id);
    case "message": return showMessage(data);
    case "settings": state.settings = data; dropTarget = data.dropRecipient || null; renderDropTarget(); applyTheme(data.theme); return renderMe();
    case "companion": state.companion = data; return;
    case "staged": return addStagedPaths(data.files || []);
    case "scan": return setScanning(data.running, data.found);
    case "focus": return flashMe();
    case "history-cleared":
      for (const [id, t] of state.transfers) if (isFinal(t)) state.transfers.delete(id);
      return renderActivity(true);
  }
}

function loadState(data) {
  state.me = data.me;
  state.settings = data.settings || {};
  dropTarget = state.settings.dropRecipient || null;
  state.companion = data.companion || {};
  state.multicast = data.multicast;
  applyTheme(state.settings.theme);
  renderMe();
  const ids = new Set((data.peers || []).map((p) => p.id));
  for (const id of [...state.peers.keys()]) if (!ids.has(id)) removePeer(id);
  for (const p of data.peers || []) upsertPeer(p, true);
  state.transfers.clear();
  for (const t of data.transfers || []) {
    state.transfers.set(t.id, t);
    lastStatus.set(t.id, t.status);
    if (t.direction === "in" && t.status === "waiting") queueIncoming(t.id);
  }
  setScanning(data.scanning);
  renderActivity(true);
  refreshPeerStates();
  layoutPeers();
}

const isFinal = (t) => !["waiting", "transferring"].includes(t.status);

/* ------------------------------------------------------------------- me */
function renderMe() {
  if (!state.me) return;
  $("meName").textContent = state.settings.alias || state.me.alias;
  $("meIp").textContent = state.me.ip || "";
}

function flashMe() {
  $("meNode").animate([{ transform: "scale(1)" }, { transform: "scale(1.1)" }, { transform: "scale(1)" }], { duration: 450, easing: "cubic-bezier(.25,.1,.25,1)" });
}

/* ---------------------------------------------------------------- peers */
function peerSubtitle(p) {
  if (p.kind === "web") return "Browser" + (p.deviceModel ? ` · ${p.deviceModel}` : "");
  if (p.kind === "quickshare") return "Quick Share";
  return p.deviceModel || (p.deviceType === "mobile" ? "Phone" : "Computer");
}

function upsertPeer(p, quiet = false) {
  const isNew = !state.peers.has(p.id);
  state.peers.set(p.id, p);
  let node = peerEls.get(p.id);
  if (!node) {
    node = el(`
      <button class="peer" data-id="${esc(p.id)}">
        <div class="bubble">
          <div class="avatar"></div>
          <svg class="ring" viewBox="0 0 100 100"><circle class="track" cx="50" cy="50" r="44"/><circle class="bar" cx="50" cy="50" r="44"/></svg>
        </div>
        <div class="label"><b></b><span></span></div>
      </button>`);
    node.addEventListener("click", (e) => onPeerClick(p.id, e));
    node.addEventListener("contextmenu", (e) => { e.preventDefault(); openPeerMenu(p.id); });
    $("peers").appendChild(node);
    peerEls.set(p.id, node);
  }
  const avatar = node.querySelector(".avatar");
  avatar.innerHTML = deviceIcon(p.deviceType, p.deviceModel);
  node.querySelector(".label b").textContent = p.alias;
  node.dataset.subtitle = peerSubtitle(p);
  node.title = `${p.alias}${p.ip ? " · " + p.ip : ""}`;
  let star = node.querySelector(".star");
  if (p.trusted && !star) node.querySelector(".bubble").appendChild(el(`<span class="star" title="Trusted">${icons.star}</span>`));
  if (!p.trusted && star) star.remove();
  node.classList.toggle("picking", state.staged.length > 0);
  refreshPeerState(p.id);
  if (isNew) layoutPeers();
  renderEmpty();
}

function removePeer(id) {
  state.peers.delete(id);
  const node = peerEls.get(id);
  peerEls.delete(id);
  if (node) {
    node.classList.add("leaving");
    setTimeout(() => node.remove(), 450);
  }
  layoutPeers();
  renderEmpty();
}

/** Keeps the grid in name order without moving nodes (which would restart animations). */
function layoutPeers() {
  const ids = [...peerEls.keys()].sort((a, b) => (state.peers.get(a)?.alias || "").localeCompare(state.peers.get(b)?.alias || "") || a.localeCompare(b));
  ids.forEach((id, i) => { peerEls.get(id).style.order = i; });
}

function renderEmpty() {
  const empty = state.peers.size === 0;
  $("empty").hidden = !empty;
  $("hint").hidden = empty || state.staged.length > 0;
  renderDropTarget();
}

/** Shows the latest transfer with a peer under its name, like the iOS share sheet. */
function refreshPeerState(id) {
  const node = peerEls.get(id);
  if (!node) return;
  const transfers = [...state.transfers.values()].filter((t) => t.peer && t.peer.id === id).sort((a, b) => b.created - a.created);
  const active = transfers.find((t) => !isFinal(t));
  const recent = transfers.find((t) => isFinal(t) && t.finished && Date.now() / 1000 - t.finished < 4.5);
  const t = active || recent;
  node.classList.remove("busy", "waiting", "success", "failed");
  const label = node.querySelector(".label span");
  const bar = node.querySelector(".ring .bar");
  if (!t) {
    label.textContent = node.dataset.subtitle;
    bar.style.strokeDashoffset = 276.5;
    return;
  }
  const progress = t.total ? t.done / t.total : 0;
  if (t.status === "waiting") {
    node.classList.add("busy", "waiting");
    label.textContent = t.direction === "out" ? "Waiting…" : "Wants to share";
    bar.style.strokeDashoffset = "";
  } else if (t.status === "transferring") {
    node.classList.add("busy");
    bar.style.strokeDashoffset = 276.5 * (1 - progress);
    label.textContent = t.direction === "out" ? "Sending…" : "Receiving…";
  } else if (t.status === "done") {
    node.classList.add("success");
    bar.style.strokeDashoffset = 0;
    label.textContent = t.direction === "out" ? "Sent" : "Received";
  } else {
    node.classList.add("failed");
    bar.style.strokeDashoffset = 276.5;
    label.textContent = t.status === "declined" ? "Declined" : t.status === "cancelled" ? "Cancelled" : "Failed";
  }
}

function refreshPeerStates() {
  for (const id of peerEls.keys()) refreshPeerState(id);
}
setInterval(refreshPeerStates, 1500); // lets "Sent ✓" fade back to the device name

/* ------------------------------------------------------------ transfers */
function updateTransfer(t) {
  const before = lastStatus.get(t.id);
  state.transfers.set(t.id, t);
  lastStatus.set(t.id, t.status);
  const peerId = t.peer && t.peer.id;
  if (before !== t.status && isFinal(t)) onFinished(t, before);
  if (t.direction === "in" && t.status !== "waiting" && currentIncoming === t.id) closeIncoming();
  if (peerId) refreshPeerState(peerId);
  renderItem(t, before !== t.status);
  updateBadge();
}

function onFinished(t, before) {
  const who = t.peer?.alias || "device";
  if (t.status === "done") {
    if (t.direction === "out") toast(t.message ? `Message sent to ${who}` : `Sent to ${who}`, summary(t), "ok");
    else if (!t.message) {
      toast(`Received from ${who}`, summary(t), "ok");
      if (!document.body.classList.contains("activity-open")) state.unseen++;
    }
  } else if (t.status === "declined" && t.direction === "out") {
    toast(`${who} declined`, summary(t), "bad");
  } else if (t.status === "failed") {
    toast(t.direction === "out" ? `Couldn't send to ${who}` : `Transfer from ${who} failed`, t.error || "", "bad", 6000);
    if (t.direction === "out" && /PIN/i.test(t.error || "")) askPin(t.peer);
  } else if (t.status === "cancelled" && before === "transferring") {
    toast("Transfer cancelled", t.error || "", "");
  }
}

function summary(t) {
  if (t.message) return t.message.length > 80 ? t.message.slice(0, 80) + "…" : t.message;
  const files = t.files || [];
  const names = files.slice(0, 2).map((f) => f.name.split("/").pop()).join(", ");
  return `${names}${files.length > 2 ? ` +${files.length - 2} more` : ""} · ${bytes(t.total)}`;
}

/* -------------------------------------------------------------- activity */
function toggleActivity(open) {
  const on = open ?? !document.body.classList.contains("activity-open");
  document.body.classList.toggle("activity-open", on);
  if (on) state.unseen = 0;
  updateBadge();
}

function updateBadge() {
  const badge = $("activityBadge");
  const active = [...state.transfers.values()].filter((t) => !isFinal(t)).length;
  const n = active || state.unseen;
  badge.hidden = n === 0 || document.body.classList.contains("activity-open");
  badge.textContent = n;
}

function renderActivity(full) {
  if (full) {
    for (const node of itemEls.values()) node.remove();
    itemEls.clear();
  }
  const list = [...state.transfers.values()].sort((a, b) => a.created - b.created);
  for (const t of list) renderItem(t, true);
  $("activityEmpty").hidden = list.length > 0;
}

function renderItem(t, structural) {
  let node = itemEls.get(t.id);
  if (!node) {
    node = el(`<div class="item"></div>`);
    itemEls.set(t.id, node);
    $("activityList").prepend(node);
    structural = true;
  }
  $("activityEmpty").hidden = true;
  const progress = t.total ? Math.min(1, t.done / t.total) : 0;
  if (!structural) {
    const bar = node.querySelector(".bar i");
    if (bar) bar.style.width = `${progress * 100}%`;
    const meta = node.querySelector(".m-status");
    if (meta) meta.textContent = statusText(t);
    return;
  }
  const who = t.peer?.alias || "Unknown device";
  const files = t.files || [];
  const title = t.message ? (t.direction === "out" ? `Message to ${who}` : `Message from ${who}`)
    : files.length === 1 ? files[0].name.split("/").pop() : `${files.length} files`;
  const pill = t.status === "done" ? `<span class="pill ok">${t.direction === "out" ? "Sent" : "Received"}</span>`
    : t.status === "transferring" ? `<span class="pill live">${t.direction === "out" ? "Sending" : "Receiving"}</span>`
    : t.status === "waiting" ? `<span class="pill live">Waiting</span>`
    : `<span class="pill bad">${t.status[0].toUpperCase() + t.status.slice(1)}</span>`;
  const images = t.status === "done" ? files.filter((f) => (f.type || "").startsWith("image/") && f.path).slice(0, 4) : [];
  const showFiles = !t.message && files.length > 1 ? files.slice(0, 4) : [];
  const hasPath = files.some((f) => f.path) && t.direction === "in";
  node.innerHTML = `
    <div class="who"><div class="avatar sm">${deviceIcon(t.peer?.deviceType, t.peer?.deviceModel)}</div>
      <span class="dir">${t.direction === "out" ? icons.arrowUp : icons.arrowDown}</span></div>
    <div class="body">
      <div class="title">${esc(title)}</div>
      <div class="meta">${pill}<span class="m-status">${esc(statusText(t))}</span></div>
    </div>
    ${t.message ? `<div class="message">${esc(t.message)}</div>` : ""}
    ${images.length ? `<div class="thumbs">${images.map((f) => `<img loading="lazy" alt="" data-file="${esc(f.id)}" src="/api/transfers/${t.id}/files/${encodeURIComponent(f.id)}/preview">`).join("")}</div>` : ""}
    ${showFiles.length ? `<div class="files">${showFiles.map((f) => `<div>${fileIcon(f.type, f.name)}<span>${esc(f.name)}</span><small class="faint">${bytes(f.size)}</small></div>`).join("")}${files.length > 4 ? `<div class="faint">+ ${files.length - 4} more</div>` : ""}</div>` : ""}
    ${!isFinal(t) && t.status === "transferring" ? `<div class="bar"><i style="width:${progress * 100}%"></i></div>` : ""}
    <div class="acts">
      ${!isFinal(t) ? `<button class="btn ghost small" data-act="cancel">${t.status === "waiting" && t.direction === "in" ? "Decline" : "Cancel"}</button>` : ""}
      ${t.direction === "in" && t.status === "waiting" ? `<button class="btn small" data-act="review">Review</button>` : ""}
      ${t.status === "done" && hasPath ? `<button class="btn ghost small" data-act="open">Open</button><button class="btn ghost small" data-act="reveal">Show in folder</button>` : ""}
      ${t.message ? `<button class="btn ghost small" data-act="copy">Copy</button>` : ""}
    </div>`;
  node.querySelectorAll("[data-act]").forEach((b) => b.addEventListener("click", () => itemAction(t.id, b.dataset.act)));
  node.querySelectorAll(".thumbs img").forEach((img) => {
    img.addEventListener("click", () => api(`/transfers/${t.id}/open`, { body: { file: img.dataset.file } }));
    img.addEventListener("error", () => img.remove());
  });
}

function statusText(t) {
  const when = ago(t.finished || t.created);
  if (t.status === "transferring") return `${bytes(t.done)} of ${bytes(t.total)} · ${bytes(t.speed)}/s`;
  if (t.status === "waiting") {
    const pin = t.peer?.pin ? ` · PIN ${t.peer.pin}` : "";
    return (t.direction === "out" ? `Waiting for ${t.peer?.alias || "device"}…` : "Waiting for you") + pin;
  }
  if (t.status === "done") return `${t.message ? "" : bytes(t.total) + " · "}${when}`;
  return `${t.error || t.status} · ${when}`;
}

async function itemAction(id, act) {
  try {
    if (act === "review") return queueIncoming(id, true);
    if (act === "copy") {
      const t = state.transfers.get(id);
      if (t?.message && (await AB.copyText(t.message))) return toast("Copied to clipboard", "", "ok", 2000);
    }
    await api(`/transfers/${id}/${act}`, { body: {} });
  } catch (err) {
    toast("Something went wrong", err.message, "bad");
  }
}

/* -------------------------------------------------------------- incoming */
function queueIncoming(id, front = false) {
  if (currentIncoming === id || incomingQueue.includes(id)) return;
  if (front) incomingQueue.unshift(id);
  else incomingQueue.push(id);
  showNextIncoming();
}

function showNextIncoming() {
  if (currentIncoming) return;
  const id = incomingQueue.shift();
  if (!id) return;
  const t = state.transfers.get(id);
  if (!t || t.status !== "waiting" || t.direction !== "in") return showNextIncoming();
  currentIncoming = id;
  const who = t.peer?.alias || "A device";
  const files = t.files || [];
  $("inPreview").innerHTML = incomingPreview(t);
  $("inTitle").textContent = "AirBridge";
  $("inSub").textContent = `“${who}” would like to share ${t.message ? "a message" : sharePhrase(files)}.`;
  $("inMeta").textContent = incomingMeta(t);
  // Quick Share shows the same four digits on the phone; there is no
  // lasting identity to trust, so no "always accept".
  const pin = t.peer?.pin;
  $("inPin").hidden = !pin;
  $("inPin").innerHTML = pin ? `PIN <b>${esc(pin)}</b> should match the phone` : "";
  $("inTrustRow").hidden = t.peer?.kind === "quickshare";
  $("inTrust").checked = false;
  $("sheetBackdrop").classList.add("open");
  $("incomingSheet").classList.add("open");
}

/** What is coming: the photo itself, a small stack of photos, or the file's icon. */
function incomingPreview(t) {
  if (t.message) return `<div class="bubble">${esc(t.message)}</div>`;
  const files = t.files || [];
  const thumbs = files.map((f) => f.thumb).filter(Boolean).slice(0, 3);
  if (thumbs.length === 1 && files.length === 1) return `<img class="photo" alt="" src="${esc(thumbs[0])}">`;
  if (thumbs.length) return `<div class="stack">${thumbs.reverse().map((src) => `<img alt="" src="${esc(src)}">`).join("")}</div>`;
  const first = files[0] || {};
  const icon = files.length > 1 && !files.every((f) => f.type === first.type) ? icons.file : fileIcon(first.type, first.name);
  return `<div class="tile">${icon}</div>`;
}

function incomingMeta(t) {
  const files = t.files || [];
  if (t.message) return "";
  const named = files.length > 1 ? files.slice(0, 2).map((f) => f.name.split("/").pop()).join(", ") + (files.length > 2 ? ` and ${files.length - 2} more` : "") : "";
  return [named, bytes(t.total)].filter(Boolean).join(" · ");
}

function closeIncoming() {
  $("sheetBackdrop").classList.remove("open");
  $("incomingSheet").classList.remove("open");
  currentIncoming = null;
  setTimeout(showNextIncoming, 300);
}

async function answerIncoming(accept) {
  const id = currentIncoming;
  if (!id) return;
  const trust = $("inTrust").checked;
  closeIncoming();
  try {
    await api(`/transfers/${id}/${accept ? "accept" : "decline"}`, { body: { trust } });
  } catch (err) {
    toast("Could not answer", err.message, "bad");
  }
}

/* -------------------------------------------------------------- messages */
function showMessage({ id, from, text, copied }) {
  const box = $("toasts");
  const node = el(`<div class="toast"><span class="t-icon">${icons.message}</span><div class="t-body">
      <div class="t-title">${esc(from)}</div><div class="t-text" style="white-space:pre-wrap;max-height:180px;overflow:auto">${esc(text)}</div>
      <div style="display:flex;gap:6px;margin-top:8px"><button class="btn small ${copied ? "ghost" : ""}" data-copy>${copied ? "Copied ✓" : "Copy"}</button><button class="btn ghost small" data-close>Dismiss</button></div>
    </div></div>`);
  const close = () => { node.classList.add("out"); setTimeout(() => node.remove(), 320); };
  node.querySelector("[data-copy]").addEventListener("click", async (e) => {
    if (await AB.copyText(text)) e.target.textContent = "Copied ✓";
  });
  node.querySelector("[data-close]").addEventListener("click", close);
  box.prepend(node);
  setTimeout(close, 14000);
}

/* ------------------------------------------------------------ peer clicks */
function onPeerClick(id, event) {
  event.stopPropagation();
  selectDropTarget(id);
  if (state.staged.length) return sendStaged(id);
}

function openPeerMenu(id) {
  const p = state.peers.get(id);
  const node = peerEls.get(id);
  if (!p || !node) return;
  const menu = $("peerMenu");
  const web = p.kind === "web";
  menu.innerHTML = `
    <div class="m-head"><div class="avatar sm">${deviceIcon(p.deviceType, p.deviceModel)}</div><div><b>${esc(p.alias)}</b><span>${esc(peerSubtitle(p))}${p.ip ? " · " + esc(p.ip) : ""}</span></div></div>
    <button class="m-item" data-act="files">${icons.file} Send Files…</button>
    <button class="m-item" data-act="folder">${icons.folder} Send a Folder…</button>
    <button class="m-item" data-act="text">${icons.message} Send a Message…</button>
    <button class="m-item" data-act="clipboard">${icons.clipboard} Send Clipboard</button>
    ${web || p.kind === "quickshare" ? "" : `<button class="m-item" data-act="trust">${icons.shield} ${p.trusted ? "Stop Trusting" : "Trust This Device"}</button>`}`;
  menu.hidden = false;
  const r = node.getBoundingClientRect();
  const mw = menu.offsetWidth;
  const mh = menu.offsetHeight;
  let left = r.left + r.width / 2 - mw / 2;
  let top = r.bottom + 6;
  if (top + mh > innerHeight - 12) top = r.top - mh - 12;
  left = Math.max(12, Math.min(left, innerWidth - mw - 12));
  menu.style.left = `${left}px`;
  menu.style.top = `${Math.max(12, top)}px`;
  menu.querySelectorAll("[data-act]").forEach((b) => b.addEventListener("click", () => {
    closePeerMenu();
    peerAction(id, b.dataset.act);
  }));
}

function closePeerMenu() {
  $("peerMenu").hidden = true;
}

async function peerAction(id, act) {
  const p = state.peers.get(id);
  try {
    if (act === "files" || act === "folder") {
      const started = performance.now();
      const res = await api("/pick", { body: { directory: act === "folder" } });
      const files = res.files || [];
      if (files.length) return sendPaths(id, files.map((f) => f.path));
      // No native chooser answered instantly → fall back to the browser's own picker.
      if (performance.now() - started < 400) return browserPick(id, act === "folder");
      return;
    }
    if (act === "text") return composeText(id);
    if (act === "clipboard") {
      await api("/send-clipboard", { body: { peer: id } });
      return;
    }
    if (act === "trust") {
      await api(`/peers/${encodeURIComponent(id)}/trust`, { body: { trusted: !p.trusted } });
      toast(p.trusted ? `${p.alias} is no longer trusted` : `${p.alias} is now trusted`, p.trusted ? "" : "Transfers from it are accepted automatically", "ok");
    }
  } catch (err) {
    toast("Something went wrong", err.message, "bad");
  }
}

function browserPick(peerId, folder) {
  const input = document.createElement("input");
  input.type = "file";
  input.multiple = true;
  if (folder) input.webkitdirectory = true;
  input.onchange = () => {
    const files = [...input.files].map((file) => ({ file, name: file.webkitRelativePath || file.name }));
    if (files.length) stageUploads(files, peerId);
  };
  input.click();
}

async function sendPaths(peerId, paths) {
  try {
    await api("/send", { body: { peer: peerId, paths, pin: state.pins[peerId] } });
  } catch (err) {
    toast("Couldn't send", err.message, "bad");
  }
}

/* ---------------------------------------------------------------- staging */
function addStagedPaths(files) {
  for (const f of files) {
    state.staged.push({ key: "p" + Math.random(), kind: "path", name: f.name, size: f.size, type: f.type, path: f.path, isDir: f.isDir, done: true, progress: 1 });
  }
  renderTray();
}

/** Uploads browser File objects into the service's staging area. */
function stageUploads(files, sendTo = null) {
  const entries = files.map(({ file, name }) => ({ key: "u" + Math.random(), kind: "upload", name, size: file.size, type: file.type, progress: 0, done: false, file }));
  state.staged.push(...entries);
  renderTray();
  const uploads = prepareUploads(entries);
  if (sendTo) sendStaged(sendTo);
  return uploads;
}

function prepareUploads(entries) {
  return Promise.all(entries.map((entry) => new Promise((resolve) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", "/api/stage");
    xhr.setRequestHeader("x-file-name", encodeURIComponent(entry.name));
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable) {
        entry.progress = e.loaded / e.total;
        updateChip(entry);
      }
    };
    xhr.onload = () => {
      try {
        const res = JSON.parse(xhr.responseText);
        if (xhr.status === 200) {
          entry.id = res.id;
          entry.done = true;
          entry.progress = 1;
        } else throw new Error(res.message);
      } catch (err) {
        entry.failed = true;
        toast(`Couldn't prepare ${entry.name}`, err.message || "", "bad");
      }
      updateChip(entry);
      resolve();
    };
    xhr.onerror = () => { entry.failed = true; toast(`Couldn't prepare ${entry.name}`, "Upload failed. Try dropping it again.", "bad"); updateChip(entry); resolve(); };
    xhr.send(entry.file);
  })));
}

async function sendStaged(peerId) {
  const peers = recipients(peerId);
  if (!peers.length) return toast("Device unavailable", "Your files are ready. Choose a nearby device.", "bad");
  const items = state.staged.splice(0);
  renderTray();
  const pending = items.filter((i) => i.kind === "upload" && !i.done);
  if (pending.length) {
    toast("Preparing files…", `${pending.length} file(s) still copying`, "", 2500);
    while (items.some((i) => i.kind === "upload" && !i.done && !i.failed)) await new Promise((r) => setTimeout(r, 120));
  }
  const paths = items.filter((i) => i.kind === "path").map((i) => i.path);
  const uploaded = items.filter((i) => i.kind === "upload" && i.id);
  const messages = items.filter((i) => i.kind === "text");
  if (!paths.length && !uploaded.length && !messages.length) return;
  for (const [index, peer] of peers.entries()) {
    // ponytail: stage a copy per recipient; share ownership if large broadcasts become common.
    const copies = index === 0 ? uploaded : uploaded.map((i) => ({ ...i, id: undefined, chip: null, done: false, failed: false }));
    if (index > 0) await prepareUploads(copies);
    const staged = copies.filter((i) => i.id).map((i) => i.id);
    try {
      if (paths.length || staged.length) await api("/send", { body: { peer, paths, staged, pin: state.pins[peer] } });
      for (const item of messages) {
        await api(item.url ? "/send-url" : "/send-text", { body: { peer, ...(item.url ? { url: item.url } : { text: item.text }), pin: state.pins[peer] } });
      }
    } catch (err) {
      toast(`Couldn't send to ${state.peers.get(peer)?.alias || "device"}`, err.message, "bad");
    }
  }
}

function clearTray() {
  for (const item of state.staged) {
    if (item.kind === "upload" && item.id) api(`/stage/${item.id}`, { method: "DELETE" }).catch(() => {});
  }
  state.staged = [];
  renderTray();
}

function renderTray() {
  const tray = $("tray");
  const items = state.staged;
  tray.hidden = items.length === 0;
  document.body.classList.toggle("staging", items.length > 0);
  for (const node of peerEls.values()) node.classList.toggle("picking", items.length > 0);
  renderEmpty();
  if (!items.length) return;
  const total = items.reduce((s, i) => s + (i.size || 0), 0);
  $("trayTitle").textContent = items.length === 1 ? items[0].name : `${items.length} items`;
  $("traySub").textContent = `${bytes(total)} · ${state.peers.size ? "choose a device" : "waiting for a device…"}`;
  $("trayFiles").innerHTML = "";
  items.forEach((item, i) => {
    const chip = el(`<div class="chip" style="animation-delay:${Math.min(i, 10) * 35}ms">${item.isDir ? icons.folder : fileIcon(item.type, item.name)}<span class="c-name">${esc(item.name)}</span><span class="c-size">${bytes(item.size)}</span><i class="c-progress"></i></div>`);
    item.chip = chip;
    $("trayFiles").appendChild(chip);
    updateChip(item);
  });
}

function updateChip(item) {
  if (!item.chip) return;
  item.chip.querySelector(".c-progress").style.width = item.done ? "0" : `${(item.progress || 0) * 100}%`;
  item.chip.style.opacity = item.failed ? 0.4 : 1;
}

/* ------------------------------------------------------------ drag & drop */
const hasFiles = (e) => !e.target?.closest?.("input,textarea,[contenteditable]") && [...(e.dataTransfer?.types || [])].some((type) => ["Files", "text/uri-list", "text/plain"].includes(type));

function peerAt(x, y) {
  const node = document.elementFromPoint(x, y)?.closest(".peer");
  if (node === $("allPeers")) return ALL_DEVICES;
  return state.peers.has(node?.dataset.id) ? node.dataset.id : null;
}

const targetNode = (id) => id === ALL_DEVICES ? $("allPeers") : peerEls.get(id);
const dropRecipientAt = (x, y) => peerAt(x, y) || dropTarget || (state.peers.size === 1 ? state.peers.keys().next().value : null);

function setHover(id) {
  if (hoverPeer === id) return;
  if (hoverPeer) targetNode(hoverPeer)?.classList.remove("drop-target");
  hoverPeer = id;
  if (id) targetNode(id)?.classList.add("drop-target");
}

function endDrag() {
  dragDepth = 0;
  document.body.classList.remove("dragging");
  for (const node of peerEls.values()) node.classList.toggle("picking", state.staged.length > 0);
  setHover(null);
}

addEventListener("dragenter", (e) => {
  if (!hasFiles(e)) return;
  e.preventDefault();
  dragDepth++;
  document.body.classList.add("dragging");
  for (const node of peerEls.values()) node.classList.add("picking");
});
addEventListener("dragover", (e) => {
  if (!hasFiles(e)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
  setHover(peerAt(e.clientX, e.clientY));
});
addEventListener("dragleave", () => {
  dragDepth--;
  if (dragDepth <= 0) endDrag();
});
addEventListener("drop", async (e) => {
  if (!hasFiles(e)) return;
  e.preventDefault();
  const target = dropRecipientAt(e.clientX, e.clientY);
  // Entries must be read synchronously, before the event ends.
  const entries = [...e.dataTransfer.items].map((i) => i.webkitGetAsEntry && i.webkitGetAsEntry()).filter(Boolean);
  const plain = [...e.dataTransfer.files];
  const url = e.dataTransfer.getData("text/uri-list").split(/\r?\n/).find((line) => line && !line.startsWith("#") && /^https?:\/\//i.test(line));
  const text = e.dataTransfer.getData("text/plain");
  endDrag();
  try {
    const files = entries.length ? await readEntries(entries) : plain.map((file) => ({ file, name: file.name }));
    if (files.length) return stageUploads(files, target);
    if (!url && !text.trim()) return;
    state.staged.push({ key: "t" + Math.random(), kind: "text", name: url || text.trim().slice(0, 60), type: "text/plain", text, url, done: true });
    renderTray();
    if (target) await sendStaged(target);
  } catch (err) {
    toast("Couldn't add drop", err.message, "bad");
  }
});

async function readEntries(entries) {
  const out = [];
  async function walk(entry, prefix) {
    if (entry.isFile) {
      const file = await new Promise((res, rej) => entry.file(res, rej));
      out.push({ file, name: prefix + file.name });
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      for (;;) {
        const batch = await new Promise((res, rej) => reader.readEntries(res, rej));
        if (!batch.length) break;
        for (const child of batch) await walk(child, `${prefix}${entry.name}/`);
      }
    }
  }
  for (const entry of entries) await walk(entry, "");
  return out;
}

/** Called by the native window (wry) with real paths: no copying needed. */
window.airbridgeNativeDrag = (x, y) => {
  if (x === null) return endDrag();
  document.body.classList.add("dragging");
  setHover(peerAt(x, y));
};

window.airbridgeNativeDrop = async (paths, x, y) => {
  const target = dropRecipientAt(x, y);
  endDrag();
  if (!paths || !paths.length) return;
  try {
    if (target && recipients(target).length) {
      for (const peer of recipients(target)) await sendPaths(peer, paths);
    }
    else await api("/stage-paths", { body: { paths } });
  } catch (err) {
    toast("Couldn't add files", err.message, "bad");
  }
};

/* ----------------------------------------------------------------- modals */
function openModal(html, onMount) {
  const modal = $("modal");
  modal.innerHTML = `<div class="modal-card" role="dialog" aria-modal="true">${html}</div>`;
  modal.hidden = false;
  modal.onclick = (e) => { if (e.target === modal) closeModal(); };
  onMount && onMount(modal.firstElementChild);
}

function closeModal() {
  $("modal").hidden = true;
  $("modal").innerHTML = "";
}

function composeText(peerId) {
  const p = state.peers.get(peerId);
  openModal(`
    <h2>Message ${esc(p?.alias || "")}</h2>
    <p class="muted">Sends text instantly — links, codes, notes.</p>
    <textarea class="input" id="msgText" placeholder="Type or paste…"></textarea>
    <div class="actions"><button class="btn ghost" data-close>Cancel</button><button class="btn" id="msgSend">${icons.send} Send</button></div>`, (card) => {
    const area = card.querySelector("#msgText");
    area.focus();
    const send = async () => {
      const text = area.value;
      if (!text.trim()) return;
      closeModal();
      try {
        await api("/send-text", { body: { peer: peerId, text, pin: state.pins[peerId] } });
      } catch (err) {
        toast("Couldn't send", err.message, "bad");
      }
    };
    card.querySelector("#msgSend").addEventListener("click", send);
    card.querySelector("[data-close]").addEventListener("click", closeModal);
    area.addEventListener("keydown", (e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) send(); });
  });
}

function askPin(peer) {
  if (!peer) return;
  openModal(`
    <h2>PIN for ${esc(peer.alias)}</h2>
    <p class="muted">This device only accepts files with a PIN. Enter the PIN shown in its settings, then send again.</p>
    <input class="input" id="pinInput" inputmode="numeric" autocomplete="off" value="${esc(state.pins[peer.id] || "")}">
    <div class="actions"><button class="btn ghost" data-close>Cancel</button><button class="btn" id="pinSave">Save PIN</button></div>`, (card) => {
    const input = card.querySelector("#pinInput");
    input.focus();
    const save = () => {
      state.pins[peer.id] = input.value.trim();
      closeModal();
      toast("PIN saved", "Send your files again", "ok");
    };
    card.querySelector("#pinSave").addEventListener("click", save);
    card.querySelector("[data-close]").addEventListener("click", closeModal);
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") save(); });
  });
}

function openPairing() {
  const url = state.companion?.url;
  if (!url) {
    openModal(`<h2>Pair a phone's browser?</h2><p class="muted">Phones without an app can share through their browser after scanning a QR code. The page talks plain HTTP on your Wi‑Fi, so it is off until you need it, and everything it sends asks you first.</p>
      <div class="actions"><button class="btn ghost" data-close>Not Now</button><button class="btn" id="turnOn">Turn On</button></div>`, (card) => {
      card.querySelector("[data-close]").addEventListener("click", closeModal);
      card.querySelector("#turnOn").addEventListener("click", async () => {
        try {
          state.settings = await api("/settings", { body: { webCompanion: true } });
          const res = await api("/state");
          state.companion = res.companion || {};
          openPairing();
        } catch (err) {
          toast("Couldn't turn it on", err.message, "bad");
        }
      });
    });
    return;
  }
  openModal(`
    <h2>Pair a phone</h2>
    <p class="muted">Best: install <b>AirBridge for Android</b> (or LocalSend). No app? Scan this with the phone's camera to share through its browser.</p>
    <div class="qr-wrap"><img alt="QR code" src="/api/qr.svg?t=${Date.now()}"></div>
    <div class="url-box"><span>${esc(url)}</span><button class="icon-btn small" id="copyUrl" title="Copy link">${icons.copy}</button></div>
    <ol class="steps"><li>Phone and computer on the same Wi‑Fi</li><li>Scan the code, keep the page open</li><li>Your phone shows up here</li></ol>
    <div class="actions"><button class="btn ghost" id="resetPairing">Reset pairing</button><button class="btn" data-close>Done</button></div>`, (card) => {
    card.querySelector("[data-close]").addEventListener("click", closeModal);
    card.querySelector("#copyUrl").addEventListener("click", async () => { if (await AB.copyText(url)) toast("Link copied", "", "ok", 1800); });
    card.querySelector("#resetPairing").addEventListener("click", async () => {
      const res = await api("/reset-pairing", { body: {} });
      state.companion.url = res.url;
      toast("Pairing reset", "Old phone links stop working", "ok");
      openPairing();
    });
  });
}

function settingSwitch(key, title, text) {
  return `<div class="switch-row"><div class="s-text"><b>${title}</b><span>${text}</span></div>
    <label class="switch"><input type="checkbox" data-key="${key}" ${state.settings[key] ? "checked" : ""}><i></i></label></div>`;
}

function openSettings() {
  const s = state.settings;
  const trusted = (s.trusted || []).map((fp) => {
    const name = state.peers.get(fp)?.alias || `Device ${fp.slice(0, 8)}…`;
    return `<div><span>${esc(name)}</span><button class="btn ghost small" data-untrust="${esc(fp)}">Remove</button></div>`;
  }).join("");
  openModal(`
    <h2>Settings</h2>
    <div class="group">
      <div class="field"><label for="sAlias">Device name</label><input class="input" id="sAlias" maxlength="64" value="${esc(s.alias)}"></div>
      <div class="field"><label for="sDir">Save received files to</label>
        <div class="dir-row"><input class="input" id="sDir" value="${esc(s.downloadDir)}"><button class="btn ghost" id="pickDir">Choose…</button></div></div>
    </div>
    <p class="section-title">Receiving</p>
    <div class="group">
      ${settingSwitch("autoAcceptTrusted", "Accept from my devices", "No prompt for devices you trust, like your own phone")}
      ${settingSwitch("openReceived", "Open what my devices send", "A photo or link from your own devices opens right away")}
      ${settingSwitch("copyReceivedText", "Copy received text", "Text from trusted devices goes straight to your clipboard")}
      ${settingSwitch("notifications", "Notifications", "Desktop notifications with Accept / Decline")}
      ${settingSwitch("quickShare", "Quick Share", "Android phones can share here from their built-in share sheet, no app needed")}
    </div>
    <p class="section-title">Sending</p>
    <div class="group">
      ${settingSwitch("edgeDrop", "Screen-edge drop zone", "Drag anything to the side of the screen and drop it on a device")}
      <div class="field"><label for="sEdge">Drop zone side</label><select class="input" id="sEdge">
        <option value="right" ${s.edgeSide !== "left" ? "selected" : ""}>Right edge</option>
        <option value="left" ${s.edgeSide === "left" ? "selected" : ""}>Left edge</option></select></div>
      ${settingSwitch("webCompanion", "Phone browser companion", "Phones without an app pair with a QR code (plain HTTP on your Wi‑Fi; what they send asks you first)")}
      <div class="field"><label for="sPin">Require a PIN (optional)</label><input class="input" id="sPin" placeholder="No PIN" value="${esc(s.pin || "")}" autocomplete="off"></div>
    </div>
    <p class="section-title">Appearance</p>
    <div class="group"><div class="field"><select class="input" id="sTheme">
      <option value="auto" ${s.theme === "auto" ? "selected" : ""}>Match system</option>
      <option value="dark" ${s.theme === "dark" ? "selected" : ""}>Dark</option>
      <option value="light" ${s.theme === "light" ? "selected" : ""}>Light</option></select></div></div>
    ${trusted ? `<p class="section-title">Trusted devices</p><div class="group trusted-list">${trusted}</div>` : ""}
    <p class="section-title">About</p>
    <p class="muted" style="margin:6px 0">AirBridge ${esc(state.me?.version || "")} · LocalSend protocol v2 · port ${esc(state.me?.port || "")}${state.multicast ? "" : " · multicast unavailable"}</p>
    <p class="fp">Fingerprint ${esc(state.me?.fingerprint || "")}</p>
    <div class="actions"><button class="btn ghost danger" id="quitApp">Quit AirBridge</button><button class="btn" data-close>Done</button></div>`, (card) => {
    const save = (changes) => api("/settings", { body: changes }).then((s2) => { state.settings = s2; renderMe(); applyTheme(s2.theme); }).catch((err) => toast("Couldn't save", err.message, "bad"));
    card.querySelector("[data-close]").addEventListener("click", closeModal);
    card.querySelector("#sAlias").addEventListener("change", (e) => save({ alias: e.target.value }));
    card.querySelector("#sDir").addEventListener("change", (e) => save({ downloadDir: e.target.value }));
    card.querySelector("#sPin").addEventListener("change", (e) => save({ pin: e.target.value }));
    card.querySelector("#sTheme").addEventListener("change", (e) => save({ theme: e.target.value }));
    card.querySelector("#sEdge").addEventListener("change", (e) => save({ edgeSide: e.target.value }));
    card.querySelectorAll("input[data-key]").forEach((input) => input.addEventListener("change", () => save({ [input.dataset.key]: input.checked })));
    card.querySelector("#pickDir").addEventListener("click", async () => {
      const res = await api("/pick", { body: { directory: true } });
      const dir = res.files?.[0]?.path;
      if (dir) {
        card.querySelector("#sDir").value = dir;
        save({ downloadDir: dir });
      }
    });
    card.querySelectorAll("[data-untrust]").forEach((b) => b.addEventListener("click", async () => {
      await api(`/peers/${encodeURIComponent(b.dataset.untrust)}/trust`, { body: { trusted: false } });
      b.parentElement.remove();
    }));
    card.querySelector("#quitApp").addEventListener("click", async () => {
      await api("/quit", { body: {} }).catch(() => {});
      document.body.innerHTML = `<div style="display:grid;place-items:center;height:100vh;color:var(--muted)">AirBridge has stopped. You can close this window.</div>`;
    });
  });
}

function renameDevice() {
  openModal(`<h2>Device name</h2><p class="muted">This is how other devices see this computer.</p>
    <input class="input" id="rename" maxlength="64" value="${esc(state.settings.alias || "")}">
    <div class="actions"><button class="btn ghost" data-close>Cancel</button><button class="btn" id="renameSave">Save</button></div>`, (card) => {
    const input = card.querySelector("#rename");
    input.select();
    const save = async () => {
      closeModal();
      const s = await api("/settings", { body: { alias: input.value } });
      state.settings = s;
      renderMe();
      toast("Renamed", `Nearby devices now see “${s.alias}”`, "ok");
    };
    card.querySelector("#renameSave").addEventListener("click", save);
    card.querySelector("[data-close]").addEventListener("click", closeModal);
    input.addEventListener("keydown", (e) => { if (e.key === "Enter") save(); });
  });
}

function setScanning(running, found) {
  $("btnScan").classList.toggle("spinning", !!running);
  if (running === false && found !== undefined) toast(found ? `Found ${found} device${found === 1 ? "" : "s"}` : "No new devices found", found ? "" : "Is the other device's app open and on this Wi‑Fi?", found ? "ok" : "");
}

async function scan() {
  setScanning(true);
  try {
    await api("/refresh", { body: {} });
    await api("/scan", { body: {} });
  } catch (err) {
    setScanning(false);
    toast("Scan failed", err.message, "bad");
  }
}

/* ------------------------------------------------------------------- init */
function init() {
  $("meNode").querySelector(".mark").innerHTML = icons.waves;
  $("btnScan").innerHTML = icons.refresh;
  $("btnPair").innerHTML = icons.qr;
  $("btnActivity").insertAdjacentHTML("afterbegin", icons.activity);
  $("btnSettings").innerHTML = icons.settings;
  $("emptyPair").innerHTML = `${icons.qr} Pair phone browser`;
  $("emptyScan").innerHTML = `${icons.refresh} Look again`;
  $("trayClear").innerHTML = icons.close;
  $("clearHistory").innerHTML = icons.trash;
  $("closeActivity").innerHTML = icons.close;
  $("dropIcon").innerHTML = icons.upload;
  $("allAvatar").innerHTML = icons.waves;
  $("allPeers").addEventListener("click", () => {
    selectDropTarget(ALL_DEVICES);
    if (state.staged.length) sendStaged(ALL_DEVICES);
  });
  $("clearDropTarget").addEventListener("click", () => selectDropTarget(null));

  $("btnScan").addEventListener("click", scan);
  $("emptyScan").addEventListener("click", scan);
  $("btnPair").addEventListener("click", openPairing);
  $("emptyPair").addEventListener("click", openPairing);
  $("btnSettings").addEventListener("click", openSettings);
  $("btnUpdate").addEventListener("click", installUpdate);
  checkUpdate();
  setInterval(checkUpdate, 6 * 60 * 60 * 1000);
  $("btnActivity").addEventListener("click", () => toggleActivity());
  $("closeActivity").addEventListener("click", () => toggleActivity(false));
  $("clearHistory").addEventListener("click", () => api("/history", { method: "DELETE" }));
  $("openDownloads").addEventListener("click", () => api("/open-downloads", { body: {} }).catch(() => {}));
  $("meChip").addEventListener("click", renameDevice);
  $("trayClear").addEventListener("click", clearTray);
  $("inAccept").addEventListener("click", () => answerIncoming(true));
  $("inDecline").addEventListener("click", () => answerIncoming(false));
  // A request needs a deliberate answer: clicking outside only nudges the alert.
  $("sheetBackdrop").addEventListener("click", () => $("incomingSheet").animate([{ transform: "translate(-50%,-50%) scale(1)" }, { transform: "translate(-50%,-50%) scale(1.03)" }, { transform: "translate(-50%,-50%) scale(1)" }], { duration: 260 }));
  document.addEventListener("click", (e) => { if (!$("peerMenu").contains(e.target)) closePeerMenu(); });
  document.addEventListener("keydown", (e) => {
    // Return accepts the request on screen, like the default button of an alert.
    if (e.key === "Enter" && currentIncoming && $("modal").hidden) return answerIncoming(true);
    if (e.key !== "Escape") return;
    if (!$("modal").hidden) return closeModal();
    if (!$("peerMenu").hidden) return closePeerMenu();
    if (state.staged.length) return clearTray();
    if (document.body.classList.contains("activity-open")) toggleActivity(false);
  });
  document.addEventListener("paste", (e) => {
    if (!$("modal").hidden) return;
    const files = [...(e.clipboardData?.files || [])];
    if (files.length) stageUploads(files.map((file) => ({ file, name: file.name || "pasted" })));
  });
  connect();
}

init();
