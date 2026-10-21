/* AirBridge shared helpers: icons, avatars, formatting and the effects canvas. */
"use strict";

const AB = (() => {
  const svg = (paths, extra = "") =>
    `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" ${extra}>${paths}</svg>`;

  const icons = {
    laptop: svg('<rect x="4" y="4.5" width="16" height="11" rx="1.8"/><path d="M2 19.5h20"/>'),
    phone: svg('<rect x="6.5" y="2.5" width="11" height="19" rx="2.6"/><path d="M11 18.2h2"/>'),
    tablet: svg('<rect x="4.5" y="2.5" width="15" height="19" rx="2.4"/><path d="M11 18.2h2"/>'),
    globe: svg('<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z"/>'),
    server: svg('<rect x="3.5" y="4" width="17" height="7" rx="1.6"/><rect x="3.5" y="13" width="17" height="7" rx="1.6"/><path d="M7 7.5h.01M7 16.5h.01"/>'),
    terminal: svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>'),
    file: svg('<path d="M14 2.5H7a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-12z"/><path d="M14 2.5V8h5"/>'),
    image: svg('<rect x="3" y="3" width="18" height="18" rx="2.5"/><circle cx="8.5" cy="8.5" r="1.6"/><path d="m21 15-4.5-4.5L5 21"/>'),
    video: svg('<rect x="2.5" y="5" width="14" height="14" rx="2.2"/><path d="m16.5 10 5-3v10l-5-3z"/>'),
    music: svg('<path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>'),
    archive: svg('<rect x="3" y="3.5" width="18" height="5" rx="1.2"/><path d="M5 8.5V19a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5M10 12.5h4"/>'),
    pdf: svg('<path d="M14 2.5H7a2 2 0 0 0-2 2v15a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-12z"/><path d="M14 2.5V8h5M8.5 15.5h7M8.5 12h4"/>'),
    apk: svg('<path d="M6 10h12v8a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2zM8 10a4 4 0 0 1 8 0M8.5 4.5l1.3 2M15.5 4.5l-1.3 2"/>'),
    text: svg('<path d="M4 6h16M4 11h16M4 16h10"/>'),
    message: svg('<path d="M21 12a8.5 8.5 0 0 1-12.6 7.4L3 21l1.6-5.2A8.5 8.5 0 1 1 21 12z"/>'),
    folder: svg('<path d="M3 7.5A2.5 2.5 0 0 1 5.5 5h3.7l2 2.2h7.3A2.5 2.5 0 0 1 21 9.7v7.8a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/>'),
    send: svg('<path d="M21.5 2.5 10.8 13.2M21.5 2.5 14.8 21.5l-4-8.3-8.3-4z"/>'),
    upload: svg('<path d="M12 16V4M6.5 9.5 12 4l5.5 5.5M4 20h16"/>'),
    download: svg('<path d="M12 4v12M6.5 10.5 12 16l5.5-5.5M4 20h16"/>'),
    settings: svg('<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>'),
    qr: svg('<rect x="3" y="3" width="7" height="7" rx="1.2"/><rect x="14" y="3" width="7" height="7" rx="1.2"/><rect x="3" y="14" width="7" height="7" rx="1.2"/><path d="M14 14h3v3M21 14v.01M17 21h4v-4M14 18v3"/>'),
    radar: svg('<path d="M19.1 4.9A10 10 0 1 0 22 12"/><path d="M15.5 8.5A5 5 0 1 0 17 12"/><path d="m12 12 7-7"/><circle cx="12" cy="12" r="1"/>'),
    refresh: svg('<path d="M20.5 12a8.5 8.5 0 1 1-2.5-6"/><path d="M20.5 3.5V9H15"/>'),
    activity: svg('<path d="M3 12h4l3 8 4-16 3 8h4"/>'),
    close: svg('<path d="M18 6 6 18M6 6l12 12"/>'),
    check: svg('<path d="m5 12.5 4.5 4.5L19 7.5"/>'),
    star: svg('<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"/>'),
    copy: svg('<rect x="8.5" y="8.5" width="12" height="12" rx="2.2"/><path d="M15.5 8.5V5.7a2.2 2.2 0 0 0-2.2-2.2H5.7a2.2 2.2 0 0 0-2.2 2.2v7.6a2.2 2.2 0 0 0 2.2 2.2h2.8"/>'),
    clipboard: svg('<rect x="5" y="4" width="14" height="17" rx="2.2"/><path d="M9 4V3h6v1M9 10h6M9 14h4"/>'),
    open: svg('<path d="M14 4h6v6M20 4l-9 9M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/>'),
    trash: svg('<path d="M4 7h16M9 7V4.5h6V7M6 7l1 13h10l1-13"/>'),
    shield: svg('<path d="M12 3 4.5 6v6c0 4.4 3.2 8.2 7.5 9 4.3-.8 7.5-4.6 7.5-9V6z"/><path d="m9 12 2 2 4-4"/>'),
    plus: svg('<path d="M12 5v14M5 12h14"/>'),
    arrowUp: svg('<path d="M12 19V5M6 11l6-6 6 6"/>'),
    arrowDown: svg('<path d="M12 5v14M6 13l6 6 6-6"/>'),
    lock: svg('<rect x="4.5" y="10.5" width="15" height="10" rx="2.2"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>'),
    sparkle: svg('<path d="M12 3v4M12 17v4M3 12h4M17 12h4M5.6 5.6l2.8 2.8M15.6 15.6l2.8 2.8M5.6 18.4l2.8-2.8M15.6 8.4l2.8-2.8"/>'),
  };

  function deviceIcon(type, model) {
    const m = (model || "").toLowerCase();
    if (type === "mobile") return /ipad|tab/.test(m) ? icons.tablet : icons.phone;
    if (type === "web") return icons.globe;
    if (type === "server") return icons.server;
    if (type === "headless") return icons.terminal;
    return icons.laptop;
  }

  function fileIcon(mime, name) {
    const n = (name || "").toLowerCase();
    if (!mime) mime = "";
    if (mime.startsWith("image/")) return icons.image;
    if (mime.startsWith("video/")) return icons.video;
    if (mime.startsWith("audio/")) return icons.music;
    if (mime === "application/pdf" || n.endsWith(".pdf")) return icons.pdf;
    if (mime.includes("android.package") || n.endsWith(".apk")) return icons.apk;
    if (/zip|tar|rar|7z|compressed|gzip/.test(mime) || /\.(zip|tar|gz|7z|rar|xz)$/.test(n)) return icons.archive;
    if (mime.startsWith("text/")) return icons.text;
    return icons.file;
  }

  /** Stable pair of hues for an id, so every device keeps its colors. */
  function hues(id) {
    let h = 2166136261;
    for (let i = 0; i < (id || "").length; i++) {
      h ^= id.charCodeAt(i);
      h = Math.imul(h, 16777619);
    }
    const a = Math.abs(h) % 360;
    const b = (a + 40 + (Math.abs(h >> 9) % 70)) % 360;
    return [a, b];
  }

  function paintAvatar(el, id) {
    const [a, b] = hues(id);
    el.style.setProperty("--a", `hsl(${a} 85% 64%)`);
    el.style.setProperty("--b", `hsl(${b} 80% 52%)`);
  }

  function bytes(n) {
    if (!n && n !== 0) return "";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = n;
    while (v >= 1000 && i < units.length - 1) {
      v /= 1000;
      i++;
    }
    return i === 0 ? `${n} B` : `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
  }

  function ago(seconds) {
    const d = Date.now() / 1000 - seconds;
    if (d < 45) return "just now";
    if (d < 3600) return `${Math.round(d / 60)} min ago`;
    if (d < 86400) return `${Math.round(d / 3600)} h ago`;
    return new Date(seconds * 1000).toLocaleDateString();
  }

  function esc(s) {
    return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  }

  function el(html) {
    const t = document.createElement("template");
    t.innerHTML = html.trim();
    return t.content.firstElementChild;
  }

  const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /* ------------------------------------------------------------ toasts */
  function toast(title, text = "", kind = "", ms = 4200) {
    const box = document.getElementById("toasts");
    if (!box) return;
    const icon = kind === "ok" ? icons.check : kind === "bad" ? icons.close : icons.sparkle;
    const node = el(`<div class="toast ${kind}"><span class="t-icon">${icon}</span><div class="t-body"><div class="t-title">${esc(title)}</div>${text ? `<div class="t-text">${esc(text)}</div>` : ""}</div></div>`);
    box.prepend(node);
    setTimeout(() => {
      node.classList.add("out");
      setTimeout(() => node.remove(), 320);
    }, ms);
  }

  /* --------------------------------------------------- effects canvas */
  // One full-window canvas for every particle effect (streams + confetti).
  const fx = {
    canvas: null,
    ctx: null,
    particles: [],
    streams: new Map(), // key -> {from(), to(), rate, color, until}
    running: false,
    dpr: 1,

    init(canvas) {
      this.canvas = canvas;
      this.ctx = canvas.getContext("2d");
      const resize = () => {
        this.dpr = window.devicePixelRatio || 1;
        canvas.width = innerWidth * this.dpr;
        canvas.height = innerHeight * this.dpr;
        canvas.style.width = innerWidth + "px";
        canvas.style.height = innerHeight + "px";
      };
      resize();
      addEventListener("resize", resize);
    },

    start() {
      if (this.running || !this.ctx) return;
      this.running = true;
      let last = performance.now();
      const frame = (now) => {
        const dt = Math.min(0.05, (now - last) / 1000);
        last = now;
        this.step(dt);
        if (this.particles.length || this.streams.size) requestAnimationFrame(frame);
        else {
          this.running = false;
          this.ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);
        }
      };
      requestAnimationFrame(frame);
    },

    /** Continuous particle stream between two moving points (a transfer). */
    stream(key, from, to, color) {
      if (reducedMotion()) return;
      const existing = this.streams.get(key);
      if (existing) {
        existing.from = from;
        existing.to = to;
        existing.color = color;
        return;
      }
      this.streams.set(key, { from, to, color, acc: 0 });
      this.start();
    },

    stopStream(key) {
      this.streams.delete(key);
    },

    /** Celebration burst at a point. */
    confetti(x, y, count = 90) {
      if (reducedMotion()) return;
      const colors = ["#7c6cff", "#ff5fa2", "#33d1ff", "#34d399", "#fbbf24", "#ffffff"];
      for (let i = 0; i < count; i++) {
        const angle = Math.random() * Math.PI * 2;
        const speed = 160 + Math.random() * 420;
        this.particles.push({
          kind: "confetti",
          x, y,
          vx: Math.cos(angle) * speed,
          vy: Math.sin(angle) * speed - 220,
          life: 1.4 + Math.random() * 0.9,
          age: 0,
          size: 4 + Math.random() * 5,
          rot: Math.random() * Math.PI,
          vr: (Math.random() - 0.5) * 14,
          color: colors[(Math.random() * colors.length) | 0],
        });
      }
      this.start();
    },

    /** A soft ring of sparks (e.g. when a device appears). */
    sparkle(x, y, color = "#33d1ff", count = 18) {
      if (reducedMotion()) return;
      for (let i = 0; i < count; i++) {
        const angle = (i / count) * Math.PI * 2;
        const speed = 70 + Math.random() * 60;
        this.particles.push({ kind: "spark", x, y, vx: Math.cos(angle) * speed, vy: Math.sin(angle) * speed, life: 0.7, age: 0, size: 2.4, color });
      }
      this.start();
    },

    step(dt) {
      const { ctx, dpr } = this;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, innerWidth, innerHeight);

      // Emit stream particles along a curved path.
      for (const s of this.streams.values()) {
        const a = s.from();
        const b = s.to();
        if (!a || !b) continue;
        s.acc += dt * 38;
        while (s.acc > 1) {
          s.acc -= 1;
          const mx = (a.x + b.x) / 2;
          const my = (a.y + b.y) / 2;
          const dx = b.x - a.x;
          const dy = b.y - a.y;
          const bend = (Math.random() - 0.5) * 0.5 + 0.22;
          this.particles.push({
            kind: "stream", t: 0, speed: 0.55 + Math.random() * 0.5,
            ax: a.x, ay: a.y, bx: b.x, by: b.y,
            cx: mx - dy * bend, cy: my + dx * bend,
            jitter: (Math.random() - 0.5) * 10,
            size: 1.6 + Math.random() * 2.2,
            color: s.color,
          });
        }
      }

      ctx.globalCompositeOperation = "lighter";
      const keep = [];
      for (const p of this.particles) {
        if (p.kind === "stream") {
          p.t += dt * p.speed;
          if (p.t >= 1) continue;
          const t = p.t;
          const u = 1 - t;
          const x = u * u * p.ax + 2 * u * t * p.cx + t * t * p.bx + p.jitter * Math.sin(t * Math.PI);
          const y = u * u * p.ay + 2 * u * t * p.cy + t * t * p.by;
          const alpha = Math.sin(t * Math.PI);
          ctx.globalAlpha = alpha * 0.9;
          ctx.fillStyle = p.color;
          ctx.shadowColor = p.color;
          ctx.shadowBlur = 12;
          ctx.beginPath();
          ctx.arc(x, y, p.size, 0, Math.PI * 2);
          ctx.fill();
          keep.push(p);
        } else {
          p.age += dt;
          if (p.age >= p.life) continue;
          p.vy += (p.kind === "confetti" ? 620 : 0) * dt;
          p.vx *= p.kind === "confetti" ? 0.985 : 0.95;
          p.vy *= p.kind === "confetti" ? 0.985 : 0.95;
          p.x += p.vx * dt;
          p.y += p.vy * dt;
          const alpha = 1 - p.age / p.life;
          ctx.globalAlpha = alpha;
          ctx.fillStyle = p.color;
          ctx.shadowBlur = p.kind === "spark" ? 10 : 0;
          ctx.shadowColor = p.color;
          if (p.kind === "confetti") {
            p.rot += p.vr * dt;
            ctx.save();
            ctx.translate(p.x, p.y);
            ctx.rotate(p.rot);
            ctx.globalCompositeOperation = "source-over";
            ctx.fillRect(-p.size / 2, -p.size / 4, p.size, p.size / 2);
            ctx.restore();
          } else {
            ctx.beginPath();
            ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2);
            ctx.fill();
          }
          keep.push(p);
        }
      }
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;
      ctx.globalCompositeOperation = "source-over";
      this.particles = keep;
    },
  };

  function center(node) {
    if (!node || !node.isConnected) return null;
    const r = node.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
  }

  /** Copy that also works on plain-HTTP pages (no async clipboard API there). */
  async function copyText(text) {
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return true;
      }
    } catch (_) { /* fall through */ }
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    let ok = false;
    try { ok = document.execCommand("copy"); } catch (_) { ok = false; }
    area.remove();
    return ok;
  }

  return { icons, deviceIcon, fileIcon, hues, paintAvatar, bytes, ago, esc, el, toast, fx, center, copyText, reducedMotion };
})();
