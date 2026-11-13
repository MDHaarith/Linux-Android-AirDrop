/* ES5 on purpose: it must run in browsers that cannot parse app.js (legacy Edge),
   and show why the window stays empty instead of failing silently. */
window.addEventListener("load", function () {
  if (window.airbridgeStarted) return;
  var note = document.getElementById("unsupported");
  if (note) note.hidden = false;
});
