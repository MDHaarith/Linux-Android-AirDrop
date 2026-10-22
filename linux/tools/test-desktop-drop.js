// Run with: node linux/tools/test-desktop-drop.js
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const calls = [];
const notices = [];
const listeners = {};
const nodes = new Map();
const node = (left, top) => ({
  dataset: {},
  hidden: false,
  closest() { return this; },
  classList: { add() {}, remove() {}, toggle() {} },
  getBoundingClientRect: () => ({ left, top, right: left + 100, bottom: top + 100 }),
});
nodes.set("allPeers", node(0, 0));
let uploadId = 0;
const context = vm.createContext({
  AB: { toast: (...args) => notices.push(args) },
  document: { getElementById: (id) => nodes.get(id), body: node(), elementFromPoint: (x, y) => [...nodes.values()].find((n) => {
    const r = n.getBoundingClientRect();
    return !n.hidden && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }) },
  window: {}, addEventListener: (name, fn) => { listeners[name] = fn; }, setInterval() {}, setTimeout,
  fetch: async (url, request) => {
    calls.push({ url, body: JSON.parse(request.body) });
    return { ok: true, json: async () => ({ id: "transfer" }) };
  },
  XMLHttpRequest: class {
    upload = {};
    open() {}
    setRequestHeader() {}
    send() {
      this.status = 200;
      this.responseText = JSON.stringify({ id: `upload-${++uploadId}` });
      queueMicrotask(() => this.onload());
    }
  },
});
const source = fs.readFileSync(require("node:path").join(__dirname, "../ui/app.js"), "utf8").replace(/\ninit\(\);\s*$/, "\n");
vm.runInContext(source, context);
vm.runInContext(`
  renderTray = () => {};
  renderDropTarget = () => {};
  state.peers.set("phone", {alias:"Phone"});
  state.peers.set("tablet", {alias:"Tablet"});
`, context);
context.phoneNode = node(0, 120);
context.tabletNode = node(0, 240);
context.phoneNode.dataset.id = "phone";
context.tabletNode.dataset.id = "tablet";
nodes.set("phone", context.phoneNode);
nodes.set("tablet", context.tabletNode);
vm.runInContext('peerEls.set("phone", phoneNode); peerEls.set("tablet", tabletNode)', context);

(async () => {
  // Native drops use their release position, even after GTK sends drag-leave.
  await vm.runInContext('hoverPeer = "phone"; window.airbridgeNativeDrop(["/tmp/photo.png"], 50, 280)', context);
  assert.equal(calls.at(-1).body.peer, "tablet");
  assert.equal(calls.filter((c) => c.url === "/api/stage-paths").length, 0);

  vm.runInContext('selectDropTarget("phone")', context);
  assert.equal(calls.at(-1).body.dropRecipient, "phone");
  await vm.runInContext('window.airbridgeNativeDrop(["/tmp/photo.png"], 500, 500)', context);
  assert.equal(calls.at(-1).body.peer, "phone");

  calls.length = 0;
  await vm.runInContext('window.airbridgeNativeDrop(["/tmp/photo.png"], 50, 50)', context);
  assert.deepEqual(calls.map((c) => c.body.peer), ["phone", "tablet"]);

  // An unavailable saved recipient never falls back to the only nearby device.
  vm.runInContext('dropTarget = "offline"; state.peers.delete("tablet")', context);
  calls.length = 0;
  await vm.runInContext('window.airbridgeNativeDrop(["/tmp/photo.png"], 500, 500)', context);
  assert.equal(calls[0].url, "/api/stage-paths");
  vm.runInContext('state.staged = [{kind:"path",path:"/tmp/photo.png"}]', context);
  await vm.runInContext('sendStaged("offline")', context);
  assert.equal(vm.runInContext("state.staged.length", context), 1);

  // Every recipient owns a distinct upload; finishing one cannot delete another's file.
  vm.runInContext(`
    state.peers.set("tablet", {alias:"Tablet"});
    state.staged = [{kind:"upload",id:"original",done:true,name:"photo.png",file:{size:12}}];
  `, context);
  calls.length = 0;
  await vm.runInContext('sendStaged(ALL_DEVICES)', context);
  assert.deepEqual(calls.map((c) => c.body.peer), ["phone", "tablet"]);
  assert.deepEqual(calls.map((c) => c.body.staged[0]), ["original", "upload-1"]);
  assert.equal(vm.runInContext("state.staged.length", context), 0);

  calls.length = 0;
  await listeners.drop({
    clientX: 50, clientY: 280, preventDefault() {},
    dataTransfer: { types: ["text/uri-list", "text/plain"], items: [], files: [],
      getData: (type) => type === "text/uri-list" ? "https://example.com/photo.png" : "Photo" },
  });
  assert.equal(calls[0].url, "/api/send-url");
  assert.equal(calls[0].body.peer, "tablet");
  assert.equal(calls[0].body.url, "https://example.com/photo.png");
  console.log("Desktop drop checks passed: release position, default, all devices, offline default, independent uploads, browser image links.");
})().catch((error) => { console.error(error); process.exitCode = 1; });
