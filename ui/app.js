// ============================================================
// app.js — frontend controller for KickDropFarmer.
// ============================================================

import { applyTranslations, t, LANGUAGES } from "./i18n.js";

const TAURI = window.__TAURI__;
const invoke = TAURI?.core?.invoke ?? (async () => {});
const listen = TAURI?.event?.listen ?? (async () => () => {});

const root = document.documentElement;
let settings = null;
let lastState = null;
let lastDiag = null;

// ---- helpers ----------------------------------------------------------

const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

function lang() {
  return settings?.language || "en";
}

function showToast(msg, isError = false) {
  const el = $("#toast");
  el.textContent = msg;
  el.classList.toggle("error", isError);
  el.classList.remove("hidden");
  void el.offsetWidth;
  el.classList.add("show");
  clearTimeout(showToast._t);
  showToast._t = setTimeout(() => el.classList.remove("show"), 2400);
}

function confirmDialog({ title, body, okLabel, cancelLabel }) {
  return new Promise((resolve) => {
    const overlay = $("#modal-overlay");
    $("#modal-title").textContent = title || "";
    $("#modal-body").textContent = body || "";
    const okBtn = $("#modal-ok");
    const cancelBtn = $("#modal-cancel");
    okBtn.textContent = okLabel || "OK";
    cancelBtn.textContent = cancelLabel || "Cancel";

    const close = (result) => {
      overlay.classList.add("hidden");
      overlay.setAttribute("aria-hidden", "true");
      okBtn.removeEventListener("click", onOk);
      cancelBtn.removeEventListener("click", onCancel);
      overlay.removeEventListener("click", onBackdrop);
      document.removeEventListener("keydown", onKey);
      resolve(result);
    };
    const onOk = () => close(true);
    const onCancel = () => close(false);
    const onBackdrop = (e) => { if (e.target === overlay) close(false); };
    const onKey = (e) => {
      if (e.key === "Escape") close(false);
      if (e.key === "Enter") close(true);
    };

    okBtn.addEventListener("click", onOk);
    cancelBtn.addEventListener("click", onCancel);
    overlay.addEventListener("click", onBackdrop);
    document.addEventListener("keydown", onKey);

    overlay.classList.remove("hidden");
    overlay.setAttribute("aria-hidden", "false");
    okBtn.focus();
  });
}

async function saveSettings() {
  try {
    await invoke("save_settings", { newSettings: settings });
  } catch (e) {
    console.error("save_settings failed", e);
  }
}

// ---- theme + language -------------------------------------------------

function applyStyle() {
  root.setAttribute("data-style", settings.ui_style);
  root.setAttribute("data-mode", settings.dark_mode ? "dark" : "light");
}

function animateModeChange(toDark) {
  const wipe = $("#theme-wipe");
  root.setAttribute("data-mode", toDark ? "dark" : "light");
  wipe.classList.remove("active");
  void wipe.offsetWidth;
  wipe.classList.add("active");
}

function applyLanguage() {
  applyTranslations(lang());
  renderAll();
}

// ---- tab navigation ---------------------------------------------------

function initTabs() {
  $$(".tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      $$(".tab-btn").forEach((b) => b.classList.remove("active"));
      $$(".tab-panel").forEach((p) => p.classList.remove("active"));
      btn.classList.add("active");
      $(`.tab-panel[data-panel="${btn.dataset.tab}"]`).classList.add("active");
      if (btn.dataset.tab === "home") requestAnimationFrame(drawAllGraphs);
    });
  });
}

// ---- settings tab -----------------------------------------------------

function bindSwitch(id, getter, setter) {
  const el = $(id);
  if (!el) return;
  const sync = () => el.classList.toggle("on", !!getter());
  const toggle = async () => {
    await setter(!getter());
    sync();
  };
  el.addEventListener("click", toggle);
  el.addEventListener("keydown", (e) => {
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      toggle();
    }
  });
  el._sync = sync;
  sync();
}

function initSettings() {
  const tokenInput = $("#set-kick-token");
  const connStatus = $("#kick-conn-status");
  const setConn = (msg, ok) => {
    if (!connStatus) return;
    connStatus.textContent = msg;
    connStatus.style.color = ok ? "var(--accent)" : "var(--accent-2)";
  };
  if (tokenInput && settings.kick_token) tokenInput.value = settings.kick_token;
  $("#kick-token-save")?.addEventListener("click", async () => {
    const tok = (tokenInput?.value || "").trim();
    if (!tok) { setConn("Paste your session_token first.", false); return; }
    setConn("Verifying…", true);
    try {
      const r = await invoke("set_kick_token", { token: tok });
      settings.kick_token = tok;
      if (r === "connected") setConn("✓ Connected to Kick.", true);
      else if (r === "token-but-api-failed") setConn("Token saved, but Kick's API rejected it — run Diagnostics (Cloudflare, or expired token).", false);
    } catch (e) { setConn(String(e), false); }
  });
  $("#kick-token-clear")?.addEventListener("click", async () => {
    if (tokenInput) tokenInput.value = "";
    try { await invoke("set_kick_token", { token: "" }); settings.kick_token = ""; setConn("Cleared.", false); } catch (_) {}
  });

  const langSel = $("#set-language");
  langSel.innerHTML = "";
  LANGUAGES.forEach((l) => {
    const opt = document.createElement("option");
    opt.value = l.code;
    opt.textContent = l.label;
    langSel.appendChild(opt);
  });
  langSel.value = settings.language;
  langSel.addEventListener("change", async () => {
    settings.language = langSel.value;
    await saveSettings();
    applyLanguage();
  });

  const styleSel = $("#set-ui-style");
  styleSel.value = settings.ui_style;
  styleSel.addEventListener("change", async () => {
    const next = styleSel.value;
    if (next === "retro" && settings.ui_style !== "retro") {
      const ok = await confirmDialog({
        title: t(lang(), "confirm_retro_title"),
        body: t(lang(), "confirm_retro_body"),
        okLabel: t(lang(), "confirm_retro_ok"),
        cancelLabel: t(lang(), "confirm_cancel"),
      });
      if (!ok) {
        styleSel.value = settings.ui_style;
        return;
      }
    }
    settings.ui_style = next;
    applyStyle();
    await saveSettings();
    renderAll();
  });

  const modeSwitch = $("#switch-mode");
  const modeLabel = $("#mode-label");
  const syncMode = () => {
    modeSwitch.classList.toggle("on", settings.dark_mode);
    modeLabel.textContent = t(lang(), settings.dark_mode ? "dark" : "light");
  };
  const toggleMode = async () => {
    settings.dark_mode = !settings.dark_mode;
    animateModeChange(settings.dark_mode);
    syncMode();
    await saveSettings();
  };
  modeSwitch.addEventListener("click", toggleMode);
  modeSwitch.addEventListener("keydown", (e) => {
    if (e.key === " " || e.key === "Enter") { e.preventDefault(); toggleMode(); }
  });
  syncMode();

  bindSwitch("#switch-autostart", () => settings.autostart, async (v) => { settings.autostart = v; await saveSettings(); });
  bindSwitch("#switch-tray", () => settings.close_to_tray, async (v) => { settings.close_to_tray = v; await saveSettings(); });
  bindSwitch("#switch-autoclaim", () => settings.auto_claim, async (v) => { settings.auto_claim = v; await saveSettings(); });
  bindSwitch("#switch-demo", () => settings.demo_mode, async (v) => { settings.demo_mode = v; await saveSettings(); });
  bindSwitch("#switch-sched", () => settings.scheduler_enabled, async (v) => { settings.scheduler_enabled = v; await saveSettings(); });
  bindSwitch("#switch-notif", () => settings.notifications_enabled, async (v) => { settings.notifications_enabled = v; await saveSettings(); });
  bindSwitch("#switch-notif-claim", () => settings.notify_on_claim, async (v) => { settings.notify_on_claim = v; await saveSettings(); });
  bindSwitch("#switch-notif-expiry", () => settings.notify_on_session_expiry, async (v) => { settings.notify_on_session_expiry = v; await saveSettings(); });
  bindSwitch("#switch-discord", () => settings.discord_notifications, async (v) => { settings.discord_notifications = v; await saveSettings(); });

  const schedStart = $("#set-sched-start");
  schedStart.value = settings.schedule_start;
  schedStart.addEventListener("change", async () => { settings.schedule_start = schedStart.value; await saveSettings(); });

  const schedEnd = $("#set-sched-end");
  schedEnd.value = settings.schedule_end;
  schedEnd.addEventListener("change", async () => { settings.schedule_end = schedEnd.value; await saveSettings(); });

  const discordUrl = $("#set-discord-url");
  discordUrl.value = settings.discord_webhook_url;
  discordUrl.addEventListener("change", async () => { settings.discord_webhook_url = discordUrl.value.trim(); await saveSettings(); });

  $("#discord-test").addEventListener("click", async () => {
    try {
      await invoke("test_discord", { url: discordUrl.value.trim() });
      showToast(t(lang(), "toast_discord_ok"));
    } catch (e) {
      showToast(t(lang(), "toast_discord_fail") + ": " + e, true);
    }
  });

  $("#reload-btn").addEventListener("click", async () => {
    await saveSettings();
    try { await invoke("reload_app"); } catch (_) { window.location.reload(); }
  });

  const diagOut = $("#diag-output");
  $("#diag-run").addEventListener("click", async () => {
    diagOut.classList.remove("hidden");
    diagOut.innerHTML = `<div class="muted small pad">${t(lang(), "diag_running")}</div>`;
    try {
      lastDiag = await invoke("run_diagnostics");
      renderDiag(lastDiag);
    } catch (e) {
      diagOut.innerHTML = `<div class="diag-probe bad"><div class="dp-note"></div></div>`;
      diagOut.querySelector(".dp-note").textContent = String(e);
    }
  });
  $("#diag-copy").addEventListener("click", async () => {
    if (!lastDiag) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(lastDiag, null, 2));
      showToast(t(lang(), "diag_copied"));
    } catch (_) {}
  });
}

function renderDiag(probes) {
  const out = $("#diag-output");
  out.innerHTML = "";
  (probes || []).forEach((p) => {
    const el = document.createElement("div");
    el.className = "diag-probe " + (p.ok ? "ok" : "bad");
    el.innerHTML =
      `<div class="dp-head"><span class="dp-name"></span><span class="dp-status"></span></div>` +
      `<div class="dp-url"></div>` +
      `<div class="dp-note"></div>` +
      (p.sample ? `<pre class="dp-sample"></pre>` : "");
    el.querySelector(".dp-name").textContent = p.name;
    el.querySelector(".dp-status").textContent = p.status ? `HTTP ${p.status}` : "ERR";
    el.querySelector(".dp-url").textContent = p.url;
    el.querySelector(".dp-note").textContent = p.note;
    if (p.sample) el.querySelector(".dp-sample").textContent = p.sample;
    out.appendChild(el);
  });
}

// ---- games tab --------------------------------------------------------

function gameByName(name) {
  return (lastState?.available_games || []).find((g) => g.name === name);
}

function renderGames() {
  const available = $("#games-available");
  const selected = $("#games-selected");
  const games = lastState?.available_games || [];
  const chosen = settings?.selected_games || [];

  const avail = games.filter((g) => !chosen.includes(g.name));
  available.innerHTML = "";
  if (avail.length === 0) {
    available.innerHTML = `<div class="muted pad">${t(lang(), "games_empty_available")}</div>`;
  } else {
    avail.forEach((g) => {
      const item = document.createElement("div");
      item.className = "game-item";
      item.innerHTML =
        `<span class="gi-name"></span>` +
        `<span class="gi-drops"></span>`;
      item.querySelector(".gi-name").textContent = g.name;
      item.querySelector(".gi-drops").textContent = `${g.total_drops} ${t(lang(), "games_drops")}`;
      item.addEventListener("click", async () => {
        settings.selected_games = [...chosen, g.name];
        await persistSelected();
      });
      available.appendChild(item);
    });
  }

  selected.innerHTML = "";
  if (chosen.length === 0) {
    selected.innerHTML = `<div class="muted pad">${t(lang(), "games_empty_selected")}</div>`;
  } else {
    chosen.forEach((name, idx) => {
      const g = gameByName(name) || { name, total_drops: 0 };
      const item = document.createElement("div");
      item.className = "game-item";
      item.draggable = true;
      item.dataset.name = name;
      item.innerHTML =
        `<span class="gi-rank">${idx + 1}</span>` +
        `<span class="gi-name"></span>` +
        `<span class="gi-drops"></span>`;
      item.querySelector(".gi-name").textContent = name;
      item.querySelector(".gi-drops").textContent = `${g.total_drops} ${t(lang(), "games_drops")}`;
      item.querySelector(".gi-name").addEventListener("click", async () => {
        settings.selected_games = chosen.filter((n) => n !== name);
        await persistSelected();
      });
      attachDrag(item, selected);
      selected.appendChild(item);
    });
  }
}

let dragSrc = null;
function attachDrag(item, container) {
  item.addEventListener("dragstart", () => {
    dragSrc = item;
    item.classList.add("dragging");
  });
  item.addEventListener("dragend", () => {
    item.classList.remove("dragging");
    $$(".game-item.drag-over").forEach((el) => el.classList.remove("drag-over"));
  });
  item.addEventListener("dragover", (e) => {
    e.preventDefault();
    item.classList.add("drag-over");
  });
  item.addEventListener("dragleave", () => item.classList.remove("drag-over"));
  item.addEventListener("drop", async (e) => {
    e.preventDefault();
    item.classList.remove("drag-over");
    if (!dragSrc || dragSrc === item) return;
    const names = Array.from(container.querySelectorAll(".game-item")).map((el) => el.dataset.name);
    const from = names.indexOf(dragSrc.dataset.name);
    const to = names.indexOf(item.dataset.name);
    names.splice(to, 0, names.splice(from, 1)[0]);
    settings.selected_games = names;
    await persistSelected();
  });
}

async function persistSelected() {
  await invoke("set_selected_games", { games: settings.selected_games });
  renderGames();
}

// ---- home tab ---------------------------------------------------------

function renderHome() {
  const banner = $("#login-banner");
  const grid = $("#stream-grid");
  const empty = $("#home-empty");
  const st = lastState;

  const loggedIn = st?.logged_in;
  banner.classList.toggle("hidden", !!loggedIn || !!st?.demo);
  if (st?.demo) {
    $("#login-banner-text").textContent = "";
  }

  const chip = $("#engine-status");
  const label = $("#engine-status-label");
  let state = "idle";
  if (st?.paused || (st && !st.in_schedule)) state = "paused";
  else if ((st?.sessions || []).length > 0) state = "farming";
  chip.setAttribute("data-state", state);
  label.textContent = t(lang(),
    state === "farming" ? "status_farming" : state === "paused" ? "status_paused" : "status_idle");
  $("#pause-toggle").textContent = t(lang(), st?.paused ? "btn_resume" : "btn_pause");

  $("#home-update").textContent = st?.last_update ? "↻ " + st.last_update : "";

  const sessions = st?.sessions || [];
  empty.classList.toggle("hidden", sessions.length > 0 || (settings.selected_games || []).length > 0);

  const totals = {};
  sessions.forEach((s) => { totals[s.game] = (totals[s.game] || 0) + 1; });
  const seen = {};
  const opLabels = sessions.map((s) => {
    seen[s.game] = (seen[s.game] || 0) + 1;
    const op = t(lang(), "home_operation");
    return totals[s.game] > 1 ? `${op} ${seen[s.game]}/${totals[s.game]}` : `${op} 1`;
  });

  const minimal = settings.ui_style === "minimal";
  grid.classList.toggle("hidden", false);

  if (minimal) {
    grid.style.display = "block";
    grid.innerHTML = "";
    sessions.forEach((s, i) => {
      const row = document.createElement("div");
      row.className = "stream-row";
      row.innerHTML =
        `<span class="sr-name"></span>` +
        `<span class="sr-bar"><span></span></span>` +
        `<span class="sr-pct"></span>`;
      row.querySelector(".sr-name").textContent = `${s.game} | ${s.channel} | ${opLabels[i]}`;
      row.querySelector(".sr-bar > span").style.width = `${s.progress_pct}%`;
      row.querySelector(".sr-pct").textContent = `${s.progress_pct.toFixed(0)}%`;
      grid.appendChild(row);
    });
  } else {
    grid.style.display = "grid";
    grid.innerHTML = "";
    sessions.forEach((s, i) => {
      const card = document.createElement("div");
      card.className = "stream-card";
      card.innerHTML =
        `<div class="sc-head">` +
          `<span class="sc-game"></span><span class="sc-sep">|</span>` +
          `<span class="sc-channel"></span><span class="sc-sep">|</span>` +
          `<span class="sc-op"></span>` +
        `</div>` +
        `<canvas id="graph-${i}"></canvas>` +
        `<div class="sc-meta"><span class="sc-status"></span><span class="sc-pct"></span></div>`;
      card.querySelector(".sc-game").textContent = s.game;
      card.querySelector(".sc-channel").textContent = s.channel;
      card.querySelector(".sc-op").textContent = opLabels[i];
      const statusKey = s.status === "offline" ? "status_offline" : s.status === "paused" ? "status_paused" : s.status === "connecting" ? "status_connecting" : "status_watching";
      const viewers = s.viewers != null ? ` · ${s.viewers.toLocaleString()} ${t(lang(), "home_viewers")}` : "";
      card.querySelector(".sc-status").textContent = t(lang(), statusKey) + viewers;
      card.querySelector(".sc-pct").textContent = `${s.progress_pct.toFixed(0)}%`;
      grid.appendChild(card);
    });
    requestAnimationFrame(drawAllGraphs);
  }

  $("#engine-log").textContent = (st?.log || []).join("\n");
  const log = $("#engine-log");
  log.scrollTop = log.scrollHeight;
}

function cssVar(name) {
  return getComputedStyle(document.body).getPropertyValue(name).trim();
}

function drawAllGraphs() {
  const sessions = lastState?.sessions || [];
  sessions.forEach((s, i) => drawGraph(`graph-${i}`, s));
}

function drawGraph(canvasId, session) {
  const canvas = document.getElementById(canvasId);
  if (!canvas) return;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight || 120;
  if (w === 0) return;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d");
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);

  const accent = cssVar("--accent") || "#00f0c0";
  const accent2 = cssVar("--accent-2") || "#ff9e2c";
  const dim = cssVar("--fg-dim") || "#4fb6a3";
  const sunk = cssVar("--bg-sunk") || "#02080a";

  ctx.fillStyle = sunk;
  ctx.fillRect(0, 0, w, h);

  const padL = 26, padB = 16, padT = 8, padR = 6;
  const gx0 = padL, gx1 = w - padR, gy0 = padT, gy1 = h - padB;

  ctx.strokeStyle = dim;
  ctx.lineWidth = 1;
  ctx.globalAlpha = 0.12;
  const vDiv = 6;
  for (let i = 0; i <= vDiv; i++) {
    const x = gx0 + (i / vDiv) * (gx1 - gx0);
    ctx.beginPath();
    ctx.moveTo(x, gy0);
    ctx.lineTo(x, gy1);
    ctx.stroke();
  }
  ctx.font = "9px monospace";
  for (let p = 0; p <= 100; p += 25) {
    const y = gy1 - (p / 100) * (gy1 - gy0);
    ctx.globalAlpha = p === 0 ? 0.4 : 0.18;
    ctx.beginPath();
    ctx.moveTo(gx0, y);
    ctx.lineTo(gx1, y);
    ctx.stroke();
    ctx.globalAlpha = 0.6;
    ctx.fillStyle = dim;
    ctx.fillText(String(p), 2, y + 3);
  }
  ctx.globalAlpha = 1;

  const hist = session.history && session.history.length ? session.history : [[0, 0]];
  const maxX = Math.max(1, hist[hist.length - 1][0]);
  const px = (mx) => gx0 + (mx / maxX) * (gx1 - gx0);
  const py = (v) => gy1 - (v / 100) * (gy1 - gy0);

  ctx.beginPath();
  ctx.moveTo(px(hist[0][0]), gy1);
  hist.forEach(([mx, v]) => ctx.lineTo(px(mx), py(v)));
  ctx.lineTo(px(hist[hist.length - 1][0]), gy1);
  ctx.closePath();
  const grad = ctx.createLinearGradient(0, gy0, 0, gy1);
  grad.addColorStop(0, withAlpha(accent, 0.34));
  grad.addColorStop(1, withAlpha(accent, 0.02));
  ctx.fillStyle = grad;
  ctx.fill();

  ctx.beginPath();
  hist.forEach(([mx, v], idx) => {
    const x = px(mx), y = py(v);
    if (idx === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  });
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2;
  ctx.lineJoin = "round";
  ctx.shadowColor = accent;
  ctx.shadowBlur = 12;
  ctx.stroke();
  ctx.shadowBlur = 0;

  const last = hist[hist.length - 1];
  const lx = px(last[0]), ly = py(last[1]);
  ctx.fillStyle = accent2;
  ctx.shadowColor = accent2;
  ctx.shadowBlur = 14;
  ctx.beginPath();
  ctx.arc(lx, ly, 3.2, 0, Math.PI * 2);
  ctx.fill();
  ctx.shadowBlur = 0;
}

function withAlpha(color, alpha) {
  color = (color || "").trim();
  if (color.startsWith("#")) {
    let h = color.slice(1);
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    const n = parseInt(h, 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
  }
  const m = color.match(/rgba?\(([^)]+)\)/);
  if (m) {
    const [r, g, b] = m[1].split(",");
    return `rgba(${(r || 0).trim()}, ${(g || 0).trim()}, ${(b || 0).trim()}, ${alpha})`;
  }
  return color;
}

// ---- inventory tab ----------------------------------------------------

function renderInventory() {
  const all = lastState?.inventory || [];
  const selected = settings.selected_games || [];
  const progWrap = $("#inv-progress");
  const claimWrap = $("#inv-claimed");
  const empty = $("#inv-empty");
  const inProgress = all.filter((d) => !d.claimed && selected.includes(d.game));
  const claimed = all.filter((d) => d.claimed);

  empty.classList.toggle("hidden", inProgress.length + claimed.length > 0);

  const card = (d) => {
    const el = document.createElement("div");
    el.className = "inv-card" + (d.claimed ? " claimed" : "");
    const pct = d.required_minutes > 0 ? Math.min(100, (d.progress_minutes / d.required_minutes) * 100) : 0;
    el.innerHTML =
      `<div class="ic-name"></div>` +
      `<div class="ic-game"></div>` +
      `<div class="inv-bar"><span style="width:${pct}%"></span></div>` +
      `<div class="ic-meta"><span>${Math.round(d.progress_minutes)}/${Math.round(d.required_minutes)} ${t(lang(), "inv_required")}</span><span>${pct.toFixed(0)}%</span></div>`;
    el.querySelector(".ic-name").textContent = d.name;
    el.querySelector(".ic-game").textContent = d.game;
    return el;
  };

  progWrap.innerHTML = "";
  claimWrap.innerHTML = "";
  inProgress.forEach((d) => progWrap.appendChild(card(d)));
  claimed.forEach((d) => claimWrap.appendChild(card(d)));
}

// ---- master render ----------------------------------------------------

function renderAll() {
  if (!settings) return;
  const demo = !!(lastState?.demo || settings.demo_mode);
  $("#demo-badge")?.classList.toggle("hidden", !demo);
  renderHome();
  renderGames();
  renderInventory();
}

// ---- bootstrap --------------------------------------------------------

const DEFAULT_SETTINGS = {
  language: "en", ui_style: "minimal", dark_mode: true, autostart: false,
  close_to_tray: true, notifications_enabled: true, notify_on_claim: true,
  notify_on_session_expiry: true, auto_claim: true, discord_notifications: false,
  discord_webhook_url: "", scheduler_enabled: false, schedule_start: "00:00",
  schedule_end: "23:59", demo_mode: false,
  selected_games: [], paused: false,
};

async function main() {
  try {
    settings = await invoke("get_settings");
  } catch (e) {
    console.error("get_settings failed; using defaults", e);
  }
  if (!settings || typeof settings !== "object") {
    settings = { ...DEFAULT_SETTINGS };
  }

  applyStyle();
  applyTranslations(lang());
  initTabs();
  initSettings();

  $("#pause-toggle").addEventListener("click", async () => {
    settings.paused = !settings.paused;
    await invoke("set_paused", { paused: settings.paused });
  });
  $("#connect-btn").addEventListener("click", () => {
    const btn = document.querySelector('.tab-btn[data-tab="settings"]');
    if (btn) btn.click();
    const inp = $("#set-kick-token");
    if (inp) { inp.scrollIntoView({ block: "center" }); inp.focus(); }
  });
  try {
    const v = await invoke("app_version");
    if (v) $("#app-version").textContent = `v${v}`;
  } catch (_) {}

  try { lastState = await invoke("get_state"); } catch (_) {}
  renderAll();

  await listen("farm-state", (event) => {
    lastState = event.payload;
    renderAll();
  });

  let resizeT;
  window.addEventListener("resize", () => {
    clearTimeout(resizeT);
    resizeT = setTimeout(drawAllGraphs, 120);
  });

  if (!TAURI) {
    window.__preview = (state) => { lastState = state; renderAll(); };
  }
}

main();
