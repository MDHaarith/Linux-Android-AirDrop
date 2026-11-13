/* The Windows screen-edge drop zone: a thin handle that opens into device
   bubbles while something is dragged onto it. Files come from the native
   window (airbridgeEdgeDrag / airbridgeEdgeDrop); text the page sees itself. */
"use strict";
/** An element the page's HTML always has. */
const $ = (id) => document.getElementById(id);
document.body.dataset.side = window.airbridgeEdgeSide || "right";
let peers = [];
let expanded = false;
let closeTimer;
async function api(path, body) {
    const res = await fetch("/api" + path, {
        method: body === undefined ? "GET" : "POST",
        headers: body === undefined ? {} : { "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: "same-origin",
    });
    let data = null;
    try {
        data = await res.json();
    }
    catch (_) { /* empty body */ }
    if (!res.ok)
        throw new Error((data && data.message) || `Request failed (${res.status})`);
    return data;
}
function subtitle(p) {
    if (p.kind === "quickshare")
        return "Quick Share";
    if (p.kind === "web")
        return "Browser";
    return p.deviceModel || (p.deviceType === "mobile" ? "Phone" : "Computer");
}
function render(list) {
    peers = list;
    const box = $("devices");
    box.textContent = "";
    if (!list.length) {
        const empty = document.createElement("p");
        empty.className = "empty";
        empty.textContent = "No devices nearby yet";
        box.append(empty);
        return;
    }
    for (const p of list) {
        const row = document.createElement("div");
        row.className = "device";
        row.dataset.id = p.id;
        row.innerHTML = AB.deviceIcon(p.deviceType, p.deviceModel);
        const text = document.createElement("div");
        const name = document.createElement("b");
        name.textContent = p.alias || "Device";
        const sub = document.createElement("span");
        sub.textContent = subtitle(p);
        text.append(name, sub);
        row.append(text);
        box.append(row);
    }
}
async function expand() {
    clearTimeout(closeTimer);
    if (expanded)
        return;
    expanded = true;
    window.ipc?.postMessage("open");
    $("handle").hidden = true;
    $("panel").hidden = false;
    $("status").textContent = "";
    api("/wake", {}).catch(() => { });
    try {
        render((await api("/state")).peers || []);
    }
    catch (_) {
        render([]);
        $("status").textContent = "AirBridge is not running";
    }
}
function collapse() {
    expanded = false;
    highlight(null);
    $("panel").hidden = true;
    $("handle").hidden = false;
    window.ipc?.postMessage("close");
}
function collapseSoon(ms = 400) {
    clearTimeout(closeTimer);
    closeTimer = window.setTimeout(collapse, ms);
}
const deviceAt = (x, y) => document.elementFromPoint(x, y)?.closest(".device")?.dataset.id || null;
function highlight(id) {
    document.querySelectorAll(".device").forEach((d) => d.classList.toggle("hover", d.dataset.id === id));
}
async function send(id, path, body) {
    highlight(null);
    if (!id)
        return collapseSoon(0);
    const name = peers.find((p) => p.id === id)?.alias || "the device";
    $("status").textContent = `Sending to ${name}…`;
    try {
        await api(path, { peer: id, ...body });
        $("status").textContent = `Sending to ${name}`;
    }
    catch (err) {
        $("status").textContent = errorText(err);
    }
    collapseSoon(1800);
}
// Files, reported by the native window in CSS pixels.
window.airbridgeEdgeDrag = (x, y) => {
    if (x === null) {
        highlight(null);
        return collapseSoon();
    }
    expand();
    highlight(deviceAt(x, y ?? 0));
};
window.airbridgeEdgeDrop = (paths, x, y) => send(deviceAt(x, y), "/send", { paths });
// Text and links dragged from other apps.
document.addEventListener("dragenter", (e) => { e.preventDefault(); expand(); });
document.addEventListener("dragover", (e) => {
    e.preventDefault();
    clearTimeout(closeTimer);
    highlight(deviceAt(e.clientX, e.clientY));
});
document.addEventListener("dragleave", (e) => {
    if (!e.relatedTarget) {
        highlight(null);
        collapseSoon();
    }
});
document.addEventListener("drop", (e) => {
    e.preventDefault();
    const text = e.dataTransfer?.getData("text/plain") || e.dataTransfer?.getData("text/uri-list");
    if (text)
        send(deviceAt(e.clientX, e.clientY), "/send-text", { text });
    else
        collapseSoon(0);
});
$("handle").addEventListener("click", () => api("/show", {}).catch(() => { }));
