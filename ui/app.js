// ============================================================
// app.js — frontend controller for KickDropFarmer.
// ============================================================

import { applyTranslations, loadLanguage, t, LANGUAGES } from "./i18n.js";

const TAURI = window.__TAURI__;
const invoke = TAURI?.core?.invoke ?? (async () => null);
const listen = TAURI?.event?.listen ?? (async () => () => {});

const root = document.documentElement;
const $ = (sel) => document.querySelector(sel);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const DEFAULT_SETTINGS = {
  language: "en", ui_style: "minimal", dark_mode: true, autostart: false,
  start_minimized: true, close_to_tray: true, notifications_enabled: true,
  notify_on_claim: true, notify_on_session_expiry: true, auto_claim: true,
  discord_notifications: false, discord_webhook_url: "", kick_token: "",
  scheduler_enabled: false, schedule_start: "00:00", schedule_end: "23:59",
  demo_mode: false, selected_games: [], paused: false,
  window_width: 1100, window_height: 720, window_maximized: false,
};

const TABS = ["home", "games", "inventory", "settings"];

let settings = { ...DEFAULT_SETTINGS };
let state = null;
let activeTab = "home";
let invScope = "selected";
let lastDiag = null;
let dragName = null;
const rendered = {};
const palette = {};
const switches = [];

// ---- helpers ----------------------------------------------------------

function store(key, value) {
  try { localStorage.setItem(key, value); } catch (_) { return; }
}

function recall(key) {
  try { return localStorage.getItem(key); } catch (_) { return null; }
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function iconButton(icon, label, onClick) {
  const btn = el("button", "btn-icon");
  btn.type = "button";
  btn.title = label;
  btn.setAttribute("aria-label", label);
  btn.appendChild(el("span", `icon icon-${icon}`));
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    onClick();
  });
  return btn;
}

function changed(key, ...parts) {
  const sig = JSON.stringify(parts);
  if (rendered[key] === sig) return false;
  rendered[key] = sig;
  return true;
}

function debounce(fn, ms) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

function formatMinutes(min) {
  const total = Math.max(0, Math.round(min));
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return t("fmt_minutes", { m });
  if (m === 0) return t("fmt_hours", { h });
  return t("fmt_hours_minutes", { h, m });
}

function formatEnds(iso) {
  const ms = Date.parse(iso || "") - Date.now();
  if (!Number.isFinite(ms)) return "";
  if (ms <= 0) return t("ends_soon");
  const hours = ms / 3600000;
  if (hours < 24) return t("ends_in_hours", { n: Math.max(1, Math.round(hours)) });
  return t("ends_in_days", { n: Math.round(hours / 24) });
}

function channelHint(preview, count) {
  if (!count) return t("any_stream");
  const names = (preview || []).join(", ");
  const extra = count - (preview || []).length;
  return extra > 0 ? t("only_on_more", { names, n: extra }) : t("only_on", { names });
}

function percent(done, total) {
  return total > 0 ? Math.min(100, (done / total) * 100) : 0;
}

function selectedGames() {
  return settings.selected_games || [];
}

function claimableCount() {
  return (state?.inventory || []).filter((d) => d.claimable && !d.claimed).length;
}

function showToast(msg, kind = "ok") {
  const box = $("#toast");
  box.textContent = msg;
  box.dataset.kind = kind;
  box.classList.remove("hidden");
  void box.offsetWidth;
  box.classList.add("show");
  clearTimeout(showToast.timer);
  showToast.timer = setTimeout(() => box.classList.remove("show"), 2800);
}

function confirmDialog({ title, body, okLabel, cancelLabel }) {
  return new Promise((resolve) => {
    const overlay = $("#modal-overlay");
    const okBtn = $("#modal-ok");
    const cancelBtn = $("#modal-cancel");
    $("#modal-title").textContent = title || "";
    $("#modal-body").textContent = body || "";
    okBtn.textContent = okLabel || t("confirm_ok");
    cancelBtn.textContent = cancelLabel || t("confirm_cancel");

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
    return true;
  } catch (e) {
    showToast(String(e), "error");
    return false;
  }
}

async function copyText(text, okKey) {
  try {
    await navigator.clipboard.writeText(text);
    showToast(t(okKey));
  } catch (_) {
    showToast(t("toast_copy_failed"), "error");
  }
}

// ---- theme ------------------------------------------------------------

function readPalette() {
  const cs = getComputedStyle(document.body);
  for (const name of ["--accent", "--accent-2", "--fg-dim", "--bg-sunk"]) {
    palette[name] = cs.getPropertyValue(name).trim();
  }
}

function applyStyle() {
  const mode = settings.dark_mode ? "dark" : "light";
  root.setAttribute("data-style", settings.ui_style);
  root.setAttribute("data-mode", mode);
  store("kdf-ui", JSON.stringify({ style: settings.ui_style, mode }));
  readPalette();
  delete rendered.graph;
}

function animateModeChange() {
  const wipe = $("#theme-wipe");
  wipe.classList.remove("active");
  void wipe.offsetWidth;
  wipe.classList.add("active");
  applyStyle();
  rerenderAll();
}

// ---- tabs -------------------------------------------------------------

function selectTab(name) {
  activeTab = TABS.includes(name) ? name : "home";
  $$(".tab-btn").forEach((b) => {
    const on = b.dataset.tab === activeTab;
    b.classList.toggle("active", on);
    b.setAttribute("aria-selected", String(on));
  });
  $$(".tab-panel").forEach((p) => p.classList.toggle("active", p.dataset.panel === activeTab));
  store("kdf-tab", activeTab);
  render();
}

function goToKickSettings() {
  selectTab("settings");
  const input = $("#set-kick-token");
  $("#kick-section").scrollIntoView({ block: "start" });
  input.focus();
}

function initTabs() {
  $$(".tab-btn").forEach((btn) => btn.addEventListener("click", () => selectTab(btn.dataset.tab)));
  document.addEventListener("keydown", (e) => {
    if (!e.ctrlKey || e.altKey || e.metaKey) return;
    const idx = Number(e.key) - 1;
    if (idx >= 0 && idx < TABS.length) {
      e.preventDefault();
      selectTab(TABS[idx]);
    }
  });
}

// ---- master render ----------------------------------------------------

function render() {
  renderChrome();
  if (activeTab === "home") renderHome();
  else if (activeTab === "games") renderGames();
  else if (activeTab === "inventory") renderInventory();
  else renderSettingsStatus();
}

function rerenderAll() {
  for (const key of Object.keys(rendered)) delete rendered[key];
  render();
}

function engineState() {
  if (!state) return "idle";
  if (state.session) return "farming";
  if (state.paused || !state.in_schedule) return "paused";
  return "idle";
}

function renderChrome() {
  const st = state || {};
  const engine = engineState();
  const ready = claimableCount();
  if (!changed("chrome", engine, st.paused, st.logged_in, st.username, st.demo, settings.demo_mode, ready)) return;
  $("#engine-status").dataset.state = engine;
  $("#engine-status-label").textContent = t(`status_${engine}`);
  $("#pause-toggle").textContent = t(st.paused ? "btn_resume" : "btn_pause");
  $("#demo-badge").classList.toggle("hidden", !(st.demo || settings.demo_mode));
  const chip = $("#account-chip");
  chip.dataset.state = st.logged_in ? "on" : "off";
  $("#account-label").textContent = st.logged_in
    ? (st.username || t("account_connected"))
    : t("account_disconnected");
  const badge = $("#claimable-count");
  badge.textContent = String(ready);
  badge.classList.toggle("hidden", ready === 0);
}

// ---- home -------------------------------------------------------------

function isConnected(st) {
  if (st.demo || st.logged_in) return true;
  return !!settings.kick_token && st.idle_reason !== "expired" && st.idle_reason !== "no_session";
}

function renderHome() {
  const st = state || {};
  const setupShown = renderSetup(st);
  renderNow(st);
  renderIdle(st, setupShown);
  renderStats(st);
  renderQueue(st);
  renderLog(st);
  $("#home-update").textContent = st.last_update ? t("updated_at", { time: st.last_update }) : "";
}

function renderSetup(st) {
  const connected = isConnected(st);
  const hasGames = selectedGames().length > 0;
  const show = !connected || !hasGames;
  if (changed("setup", show, connected, hasGames)) {
    $("#setup-card").classList.toggle("hidden", !show);
    $("#setup-step-connect").classList.toggle("done", connected);
    $("#setup-step-games").classList.toggle("done", hasGames);
    $("#setup-connect").classList.toggle("hidden", connected);
    $("#setup-games").classList.toggle("hidden", hasGames);
    $("#setup-demo").classList.toggle("hidden", connected);
  }
  return show;
}

function renderNow(st) {
  const s = st.session;
  $("#now-card").classList.toggle("hidden", !s);
  if (!s) {
    delete rendered.now;
    return;
  }
  if (changed("now", s.drop_id, s.drop_name, s.channel, s.game, s.progress_minutes, s.required_minutes, s.status, s.viewers, s.drop_image)) {
    $("#now-drop").textContent = s.drop_name;
    $("#now-game").textContent = s.game;
    const channel = $("#now-channel");
    channel.textContent = s.channel;
    channel.dataset.slug = s.channel;
    channel.disabled = !!st.demo;
    const img = $("#now-image");
    if (s.drop_image) {
      if (img.dataset.src !== s.drop_image) {
        img.dataset.src = s.drop_image;
        img.src = s.drop_image;
      }
      img.classList.remove("hidden");
    } else {
      img.classList.add("hidden");
    }
    const pct = percent(s.progress_minutes, s.required_minutes);
    $("#now-bar").style.width = `${pct}%`;
    $("#now-progress").textContent = t("progress_of", {
      done: formatMinutes(s.progress_minutes),
      total: formatMinutes(s.required_minutes),
      pct: pct.toFixed(0),
    });
    const left = s.required_minutes - s.progress_minutes;
    $("#now-eta").textContent = left > 0 ? t("eta_left", { time: formatMinutes(left) }) : t("ready_to_claim");
    const pill = $("#now-status");
    pill.dataset.state = s.status;
    pill.textContent = t(`session_${s.status}`);
    $("#now-viewers").textContent = s.viewers != null ? t("viewers_count", { n: s.viewers.toLocaleString() }) : "";
  }
  drawGraph(s);
}

const IDLE = {
  paused: { action: "btn_resume", run: () => togglePause(false) },
  schedule: { action: "idle_open_settings", run: () => selectTab("settings") },
  expired: { action: "idle_reconnect", run: goToKickSettings },
  no_live: null,
  all_claimed: { action: "setup_games_btn", run: () => selectTab("games") },
  no_campaigns: { action: "setup_games_btn", run: () => selectTab("games") },
  loading: null,
};

function renderIdle(st, setupShown) {
  const reason = st.session || setupShown ? null : (st.idle_reason || "loading");
  const card = $("#idle-card");
  card.classList.toggle("hidden", !reason || !(reason in IDLE));
  if (!reason || !(reason in IDLE)) return;
  if (!changed("idle", reason, settings.schedule_start)) return;
  $("#idle-title").textContent = t(`idle_${reason}_title`);
  $("#idle-body").textContent = t(`idle_${reason}_body`, { start: settings.schedule_start });
  const action = $("#idle-action");
  const spec = IDLE[reason];
  action.classList.toggle("hidden", !spec);
  if (spec) {
    action.textContent = t(spec.action);
    action.onclick = spec.run;
  }
}

function renderStats(st) {
  const chosen = selectedGames();
  const left = (st.inventory || []).filter((d) => chosen.includes(d.game) && !d.claimed).length;
  if (!changed("stats", left, st.claimed_count, Math.floor((st.watch_seconds || 0) / 60))) return;
  $("#stat-unclaimed").textContent = String(left);
  $("#stat-claimed").textContent = String(st.claimed_count || 0);
  $("#stat-watch").textContent = formatMinutes((st.watch_seconds || 0) / 60);
}

function renderQueue(st) {
  const queue = st.queue || [];
  $("#queue-section").classList.toggle("hidden", queue.length === 0);
  if (!changed("queue", queue)) return;
  const box = $("#queue");
  box.replaceChildren(...queue.map((q, i) => {
    const row = el("div", "queue-item");
    row.appendChild(el("span", "q-rank", String(i + 1)));
    const body = el("div", "q-body");
    body.appendChild(el("span", "q-name", q.drop_name));
    body.appendChild(el("span", "q-meta muted small", `${q.game} | ${channelHint(q.channel_preview, q.channel_count)}`));
    row.appendChild(body);
    row.appendChild(el("span", "q-left small", t("eta_left", { time: formatMinutes(q.remaining_minutes) })));
    return row;
  }));
}

function renderLog(st) {
  const log = st.log || [];
  if (!changed("log", log.length, log[log.length - 1] || "")) return;
  const box = $("#engine-log");
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 24;
  box.textContent = log.length ? log.join("\n") : t("log_empty");
  if (atBottom) box.scrollTop = box.scrollHeight;
}

function withAlpha(color, alpha) {
  const c = (color || "").trim();
  if (c.startsWith("#")) {
    let h = c.slice(1);
    if (h.length === 3) h = h.split("").map((x) => x + x).join("");
    const n = parseInt(h.slice(0, 6), 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
  }
  return c;
}

function drawGraph(session) {
  const canvas = $("#now-graph");
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (!session || w === 0 || h === 0) return;
  const hist = session.history?.length ? session.history : [[0, session.progress_pct || 0]];
  const last = hist[hist.length - 1];
  if (!changed("graph", w, h, hist.length, last)) return;

  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(h * dpr);
  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

  const accent = palette["--accent"] || "#00f0c0";
  const accent2 = palette["--accent-2"] || "#ff9e2c";
  const dim = palette["--fg-dim"] || "#4fb6a3";
  ctx.fillStyle = palette["--bg-sunk"] || "#02080a";
  ctx.fillRect(0, 0, w, h);

  const padL = 26, padB = 16, padT = 8, padR = 8;
  const gx0 = padL, gx1 = w - padR, gy0 = padT, gy1 = h - padB;

  ctx.strokeStyle = dim;
  ctx.fillStyle = dim;
  ctx.lineWidth = 1;
  ctx.font = "9px monospace";
  for (let p = 0; p <= 100; p += 25) {
    const y = gy1 - (p / 100) * (gy1 - gy0);
    ctx.globalAlpha = p === 0 ? 0.4 : 0.15;
    ctx.beginPath();
    ctx.moveTo(gx0, y);
    ctx.lineTo(gx1, y);
    ctx.stroke();
    ctx.globalAlpha = 0.6;
    ctx.fillText(String(p), 2, y + 3);
  }
  ctx.globalAlpha = 1;

  const minX = hist[0][0];
  const span = Math.max(1, last[0] - minX);
  const px = (mx) => gx0 + ((mx - minX) / span) * (gx1 - gx0);
  const py = (v) => gy1 - (Math.min(100, v) / 100) * (gy1 - gy0);

  ctx.beginPath();
  ctx.moveTo(px(hist[0][0]), gy1);
  hist.forEach(([mx, v]) => ctx.lineTo(px(mx), py(v)));
  ctx.lineTo(px(last[0]), gy1);
  ctx.closePath();
  const grad = ctx.createLinearGradient(0, gy0, 0, gy1);
  grad.addColorStop(0, withAlpha(accent, 0.3));
  grad.addColorStop(1, withAlpha(accent, 0.02));
  ctx.fillStyle = grad;
  ctx.fill();

  ctx.lineJoin = "round";
  for (const [width, alpha] of [[6, 0.18], [2, 1]]) {
    ctx.beginPath();
    hist.forEach(([mx, v], i) => (i === 0 ? ctx.moveTo(px(mx), py(v)) : ctx.lineTo(px(mx), py(v))));
    ctx.strokeStyle = withAlpha(accent, alpha);
    ctx.lineWidth = width;
    ctx.stroke();
  }

  ctx.fillStyle = accent2;
  ctx.beginPath();
  ctx.arc(px(last[0]), py(last[1]), 3.5, 0, Math.PI * 2);
  ctx.fill();
}

async function togglePause(next) {
  const paused = next ?? !state?.paused;
  if (state) state.paused = paused;
  renderChrome();
  try {
    await invoke("set_paused", { paused });
  } catch (e) {
    showToast(String(e), "error");
  }
}

function initHome() {
  $("#pause-toggle").addEventListener("click", () => togglePause());
  $("#account-chip").addEventListener("click", goToKickSettings);
  $("#setup-connect").addEventListener("click", goToKickSettings);
  $("#setup-games").addEventListener("click", () => selectTab("games"));
  $("#setup-demo").addEventListener("click", async () => {
    settings.demo_mode = true;
    syncSwitches();
    await saveSettings();
  });
  $("#now-channel").addEventListener("click", async (e) => {
    const slug = e.currentTarget.dataset.slug;
    if (!slug) return;
    try { await invoke("open_channel", { slug }); } catch (err) { showToast(String(err), "error"); }
  });
  $("#now-image").addEventListener("error", (e) => e.currentTarget.classList.add("hidden"));
  $("#log-copy").addEventListener("click", () => copyText((state?.log || []).join("\n"), "toast_log_copied"));
}

// ---- games ------------------------------------------------------------

function gameMeta(g) {
  const parts = [];
  if (g) {
    parts.push(t("games_unclaimed", { n: g.unclaimed_drops, total: g.total_drops }));
    const ends = formatEnds(g.ends_at);
    if (ends) parts.push(ends);
  } else {
    parts.push(t("games_no_campaign"));
  }
  return parts.join(" | ");
}

async function persistSelected(next) {
  settings.selected_games = next;
  delete rendered.games;
  renderGames();
  try {
    await invoke("set_selected_games", { games: next });
  } catch (e) {
    showToast(String(e), "error");
  }
}

function moveGame(name, delta) {
  const list = [...selectedGames()];
  const from = list.indexOf(name);
  const to = from + delta;
  if (from < 0 || to < 0 || to >= list.length) return;
  list.splice(to, 0, list.splice(from, 1)[0]);
  persistSelected(list);
}

function renderGames() {
  if (dragName) return;
  const games = state?.available_games || [];
  const chosen = selectedGames();
  const query = $("#games-search").value.trim().toLowerCase();
  if (!changed("games", games, chosen, query, !!state?.loaded)) return;

  const avail = games.filter((g) => !chosen.includes(g.name) && g.name.toLowerCase().includes(query));
  const availBox = $("#games-available");
  if (avail.length === 0) {
    let key = "games_none";
    if (!state?.loaded) key = "games_loading";
    else if (query) key = "games_no_match";
    else if (games.length > 0) key = "games_all_selected";
    availBox.replaceChildren(el("div", "muted pad", t(key)));
  } else {
    availBox.replaceChildren(...avail.map((g) => {
      const row = el("div", "game-item");
      row.tabIndex = 0;
      const body = el("div", "gi-body");
      body.appendChild(el("span", "gi-name", g.name));
      body.appendChild(el("span", "gi-meta", gameMeta(g)));
      row.appendChild(body);
      const add = () => persistSelected([...chosen, g.name]);
      row.appendChild(iconButton("plus", t("games_add"), add));
      row.addEventListener("click", add);
      row.addEventListener("keydown", (e) => { if (e.key === "Enter") add(); });
      return row;
    }));
  }

  const selBox = $("#games-selected");
  if (chosen.length === 0) {
    selBox.replaceChildren(el("div", "muted pad", t("games_empty_selected")));
    return;
  }
  selBox.replaceChildren(...chosen.map((name, idx) => {
    const g = games.find((x) => x.name === name);
    const row = el("div", "game-item selected" + (g ? "" : " stale"));
    row.draggable = true;
    row.dataset.name = name;
    row.appendChild(el("span", "icon icon-grip"));
    row.appendChild(el("span", "gi-rank", String(idx + 1)));
    const body = el("div", "gi-body");
    body.appendChild(el("span", "gi-name", name));
    body.appendChild(el("span", "gi-meta", gameMeta(g)));
    row.appendChild(body);
    const actions = el("div", "gi-actions");
    const up = iconButton("up", t("games_move_up"), () => moveGame(name, -1));
    const down = iconButton("down", t("games_move_down"), () => moveGame(name, 1));
    up.disabled = idx === 0;
    down.disabled = idx === chosen.length - 1;
    actions.append(up, down, iconButton("x", t("games_remove"), () => persistSelected(chosen.filter((n) => n !== name))));
    row.appendChild(actions);
    attachDrag(row);
    return row;
  }));
}

function attachDrag(row) {
  row.addEventListener("dragstart", (e) => {
    dragName = row.dataset.name;
    row.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
  });
  row.addEventListener("dragend", () => {
    dragName = null;
    $$(".game-item.dragging, .game-item.drag-over").forEach((n) => n.classList.remove("dragging", "drag-over"));
    delete rendered.games;
    renderGames();
  });
  row.addEventListener("dragover", (e) => {
    e.preventDefault();
    row.classList.add("drag-over");
  });
  row.addEventListener("dragleave", () => row.classList.remove("drag-over"));
  row.addEventListener("drop", (e) => {
    e.preventDefault();
    const source = dragName;
    const target = row.dataset.name;
    dragName = null;
    if (!source || source === target) return;
    const list = [...selectedGames()];
    const from = list.indexOf(source);
    const to = list.indexOf(target);
    if (from < 0 || to < 0) return;
    list.splice(to, 0, list.splice(from, 1)[0]);
    persistSelected(list);
  });
}

function initGames() {
  $("#games-search").addEventListener("input", debounce(renderGames, 120));
}

// ---- inventory --------------------------------------------------------

const cardCache = new Map();

function cachedCard(d) {
  const sig = JSON.stringify([d, settings.language, Math.floor(Date.now() / 3600000)]);
  const hit = cardCache.get(d.id);
  if (hit && hit.sig === sig) return hit.node;
  const node = inventoryCard(d);
  cardCache.set(d.id, { sig, node });
  return node;
}

function inventoryCard(d) {
  const card = el("div", "inv-card" + (d.claimed ? " claimed" : d.claimable ? " ready" : ""));
  if (d.image) {
    const img = el("img", "ic-img");
    img.loading = "lazy";
    img.alt = "";
    img.src = d.image;
    img.addEventListener("error", () => img.remove());
    card.appendChild(img);
  }
  const body = el("div", "ic-body");
  body.appendChild(el("div", "ic-name", d.name));
  const ends = d.claimed ? "" : formatEnds(d.ends_at);
  body.appendChild(el("div", "ic-game", ends ? `${d.game} | ${ends}` : d.game));
  const pct = percent(d.progress_minutes, d.required_minutes);
  const bar = el("div", "progress");
  const fill = el("span");
  fill.style.width = `${pct}%`;
  bar.appendChild(fill);
  body.appendChild(bar);
  const meta = el("div", "ic-meta");
  meta.appendChild(el("span", null, t("progress_short", {
    done: formatMinutes(Math.min(d.progress_minutes, d.required_minutes)),
    total: formatMinutes(d.required_minutes),
  })));
  meta.appendChild(el("span", "ic-pct", d.claimed ? t("inv_claimed_tag") : `${pct.toFixed(0)}%`));
  body.appendChild(meta);
  if (!d.claimed) body.appendChild(el("div", "ic-hint small muted", channelHint(d.channel_preview, d.channel_count)));
  if (d.claimable && !d.claimed) {
    const btn = el("button", "btn-primary small ic-claim", t("btn_claim"));
    btn.type = "button";
    btn.addEventListener("click", async () => {
      btn.disabled = true;
      try {
        await invoke("claim_drop", { id: d.id });
        showToast(t("toast_claimed", { name: d.name }));
      } catch (e) {
        btn.disabled = false;
        showToast(`${t("toast_claim_failed")}: ${e}`, "error");
      }
    });
    body.appendChild(btn);
  }
  card.appendChild(body);
  return card;
}

function inventorySection(titleKey, items, collapsible) {
  const wrap = el(collapsible ? "details" : "section", "inv-section");
  const head = el(collapsible ? "summary" : "h3", "group-label", `${t(titleKey)} (${items.length})`);
  wrap.appendChild(head);
  const grid = el("div", "inv-grid");
  grid.replaceChildren(...items.map(cachedCard));
  wrap.appendChild(grid);
  if (collapsible) {
    wrap.open = recall("kdf-claimed-open") === "1";
    wrap.addEventListener("toggle", () => store("kdf-claimed-open", wrap.open ? "1" : "0"));
  }
  return wrap;
}

function renderInventory() {
  const all = state?.inventory || [];
  const chosen = selectedGames();
  const items = invScope === "all" ? all : all.filter((d) => chosen.includes(d.game));
  if (!changed("inventory", items, invScope, chosen)) return;

  const rank = (d) => {
    const i = chosen.indexOf(d.game);
    return i < 0 ? chosen.length : i;
  };
  const ready = items.filter((d) => d.claimable && !d.claimed);
  const active = items
    .filter((d) => !d.claimed && !d.claimable)
    .sort((a, b) => rank(a) - rank(b) || (a.required_minutes - a.progress_minutes) - (b.required_minutes - b.progress_minutes));
  const claimed = items.filter((d) => d.claimed);
  const live = new Set(all.map((d) => d.id));
  for (const id of cardCache.keys()) if (!live.has(id)) cardCache.delete(id);

  const sections = [];
  if (ready.length) sections.push(inventorySection("inv_ready", ready, false));
  if (active.length) sections.push(inventorySection("inv_in_progress", active, false));
  if (claimed.length) sections.push(inventorySection("inv_claimed", claimed, true));
  $("#inv-list").replaceChildren(...sections);

  const empty = $("#inv-empty");
  empty.classList.toggle("hidden", sections.length > 0);
  if (!sections.length) {
    empty.textContent = t(invScope === "selected" && chosen.length === 0 ? "inv_empty_select" : "inv_empty");
  }
}

function initInventory() {
  $$(".seg-btn").forEach((btn) => btn.addEventListener("click", () => {
    invScope = btn.dataset.scope;
    store("kdf-inv-scope", invScope);
    $$(".seg-btn").forEach((b) => b.classList.toggle("active", b === btn));
    renderInventory();
  }));
  const saved = recall("kdf-inv-scope");
  if (saved === "all" || saved === "selected") {
    invScope = saved;
    $$(".seg-btn").forEach((b) => b.classList.toggle("active", b.dataset.scope === saved));
  }
}

// ---- settings ---------------------------------------------------------

function bindSwitch(id, key, { enabledWhen, after } = {}) {
  const node = $(id);
  const sync = () => {
    node.classList.toggle("on", !!settings[key]);
    node.setAttribute("aria-checked", String(!!settings[key]));
    const enabled = enabledWhen ? enabledWhen() : true;
    node.setAttribute("aria-disabled", String(!enabled));
    node.closest(".setting-row")?.classList.toggle("disabled", !enabled);
  };
  const toggle = async () => {
    if (node.getAttribute("aria-disabled") === "true") return;
    const prev = settings[key];
    settings[key] = !prev;
    syncSwitches();
    if (!(await saveSettings())) {
      settings[key] = prev;
      syncSwitches();
      return;
    }
    if (after) after();
  };
  node.addEventListener("click", toggle);
  node.addEventListener("keydown", (e) => {
    if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      toggle();
    }
  });
  switches.push(sync);
}

function syncSwitches() {
  switches.forEach((sync) => sync());
  const sched = settings.scheduler_enabled;
  $("#set-sched-start").disabled = !sched;
  $("#set-sched-end").disabled = !sched;
  $("#set-sched-start").closest(".setting-row").classList.toggle("disabled", !sched);
  $("#set-sched-end").closest(".setting-row").classList.toggle("disabled", !sched);
  $("#mode-label").textContent = t(settings.dark_mode ? "dark" : "light");
}

function syncSettingsInputs() {
  $("#set-language").value = settings.language;
  $("#set-ui-style").value = settings.ui_style;
  $("#set-sched-start").value = settings.schedule_start;
  $("#set-sched-end").value = settings.schedule_end;
  $("#set-discord-url").value = settings.discord_webhook_url;
  $("#set-kick-token").value = settings.kick_token || "";
  syncSwitches();
}

function setConnMessage(key, kind) {
  const msg = $("#kick-conn-status");
  msg.textContent = key ? t(key) : "";
  msg.dataset.kind = kind || "";
}

function renderSettingsStatus() {
  const st = state || {};
  if (!changed("conn", st.logged_in, st.username, st.idle_reason, st.demo, !!settings.kick_token)) return;
  const box = $("#kick-conn-state");
  let key = "conn_none";
  let kind = "off";
  if (st.demo) { key = "conn_demo"; kind = "demo"; }
  else if (st.logged_in) { key = "conn_ok"; kind = "on"; }
  else if (st.idle_reason === "expired") { key = "conn_expired"; kind = "bad"; }
  else if (settings.kick_token) { key = "conn_checking"; kind = "wait"; }
  box.dataset.state = kind;
  box.textContent = t(key, { name: st.username || t("account_connected") });
  $("#kick-token-clear").disabled = !settings.kick_token;
}

function syncReveal() {
  const hidden = $("#set-kick-token").type === "password";
  $("#kick-token-reveal").textContent = t(hidden ? "btn_show" : "btn_hide");
}

function initKickConnection() {
  const input = $("#set-kick-token");
  $("#kick-token-reveal").addEventListener("click", () => {
    input.type = input.type === "password" ? "text" : "password";
    syncReveal();
  });
  $("#kick-open").addEventListener("click", async () => {
    try { await invoke("open_kick"); } catch (e) { showToast(String(e), "error"); }
  });
  $("#kick-token-save").addEventListener("click", async () => {
    const value = input.value.trim();
    if (!value) {
      setConnMessage("conn_msg_empty", "bad");
      input.focus();
      return;
    }
    const btn = $("#kick-token-save");
    btn.disabled = true;
    setConnMessage("conn_msg_verifying", "wait");
    try {
      const result = await invoke("set_kick_token", { token: value });
      settings.kick_token = value;
      settings.demo_mode = false;
      syncSwitches();
      const map = {
        connected: ["conn_msg_connected", "ok"],
        partial: ["conn_msg_partial", "warn"],
        rejected: ["conn_msg_rejected", "bad"],
        unverified: ["conn_msg_unverified", "warn"],
      };
      const [key, kind] = map[result] || ["conn_msg_connected", "ok"];
      setConnMessage(key, kind);
    } catch (e) {
      setConnMessage(null);
      $("#kick-conn-status").textContent = String(e);
    } finally {
      btn.disabled = false;
      delete rendered.conn;
      render();
    }
  });
  $("#kick-token-clear").addEventListener("click", async () => {
    const ok = await confirmDialog({
      title: t("confirm_disconnect_title"),
      body: t("confirm_disconnect_body"),
      okLabel: t("btn_disconnect"),
    });
    if (!ok) return;
    try {
      await invoke("set_kick_token", { token: "" });
      settings.kick_token = "";
      input.value = "";
      setConnMessage("conn_msg_cleared", "ok");
    } catch (e) {
      showToast(String(e), "error");
    }
    delete rendered.conn;
    render();
  });
}

function renderDiag(probes) {
  const out = $("#diag-output");
  out.replaceChildren(...(probes || []).map((p) => {
    const item = el("div", "diag-probe " + (p.ok ? "ok" : "bad"));
    const head = el("div", "dp-head");
    head.appendChild(el("span", "dp-name", p.name));
    head.appendChild(el("span", "dp-status", p.status ? `HTTP ${p.status}` : (p.ok ? "OK" : "ERR")));
    item.appendChild(head);
    item.appendChild(el("div", "dp-url", p.url));
    item.appendChild(el("div", "dp-note", p.note));
    if (p.sample) item.appendChild(el("pre", "dp-sample", p.sample));
    return item;
  }));
}

function initSettings() {
  const langSel = $("#set-language");
  langSel.replaceChildren(...LANGUAGES.map((l) => {
    const opt = el("option", null, l.label);
    opt.value = l.code;
    return opt;
  }));
  langSel.addEventListener("change", async () => {
    settings.language = langSel.value;
    await loadLanguage(settings.language);
    applyTranslations();
    syncSwitches();
    syncReveal();
    rerenderAll();
    await saveSettings();
  });

  const styleSel = $("#set-ui-style");
  styleSel.addEventListener("change", async () => {
    const next = styleSel.value;
    if (next === "retro" && settings.ui_style !== "retro") {
      const ok = await confirmDialog({
        title: t("confirm_retro_title"),
        body: t("confirm_retro_body"),
        okLabel: t("confirm_retro_ok"),
      });
      if (!ok) {
        styleSel.value = settings.ui_style;
        return;
      }
    }
    settings.ui_style = next;
    applyStyle();
    rerenderAll();
    await saveSettings();
  });

  bindSwitch("#switch-mode", "dark_mode", { after: animateModeChange });
  bindSwitch("#switch-autostart", "autostart");
  bindSwitch("#switch-minimized", "start_minimized", { enabledWhen: () => settings.autostart });
  bindSwitch("#switch-tray", "close_to_tray");
  bindSwitch("#switch-autoclaim", "auto_claim");
  bindSwitch("#switch-demo", "demo_mode");
  bindSwitch("#switch-sched", "scheduler_enabled");
  bindSwitch("#switch-notif", "notifications_enabled");
  bindSwitch("#switch-notif-claim", "notify_on_claim", { enabledWhen: () => settings.notifications_enabled });
  bindSwitch("#switch-notif-expiry", "notify_on_session_expiry", { enabledWhen: () => settings.notifications_enabled });
  bindSwitch("#switch-discord", "discord_notifications");

  const schedStart = $("#set-sched-start");
  schedStart.addEventListener("change", async () => { settings.schedule_start = schedStart.value || "00:00"; await saveSettings(); });
  const schedEnd = $("#set-sched-end");
  schedEnd.addEventListener("change", async () => { settings.schedule_end = schedEnd.value || "23:59"; await saveSettings(); });

  const discordUrl = $("#set-discord-url");
  discordUrl.addEventListener("change", async () => { settings.discord_webhook_url = discordUrl.value.trim(); await saveSettings(); });
  $("#discord-test").addEventListener("click", async () => {
    try {
      await invoke("test_discord", { url: discordUrl.value.trim() });
      showToast(t("toast_discord_ok"));
    } catch (e) {
      showToast(`${t("toast_discord_fail")}: ${e}`, "error");
    }
  });

  const diagOut = $("#diag-output");
  $("#diag-run").addEventListener("click", async () => {
    const btn = $("#diag-run");
    btn.disabled = true;
    diagOut.classList.remove("hidden");
    diagOut.replaceChildren(el("div", "muted small pad", t("diag_running")));
    try {
      lastDiag = await invoke("run_diagnostics");
      renderDiag(lastDiag);
    } catch (e) {
      renderDiag([{ name: "diagnostics", url: "", status: 0, ok: false, note: String(e), sample: "" }]);
    } finally {
      btn.disabled = false;
    }
  });
  $("#diag-copy").addEventListener("click", () => {
    if (lastDiag) copyText(JSON.stringify(lastDiag, null, 2), "diag_copied");
  });

  $("#reload-btn").addEventListener("click", () => window.location.reload());

  initKickConnection();
  syncSettingsInputs();
}

// ---- bootstrap --------------------------------------------------------

async function main() {
  try {
    const loaded = await invoke("get_settings");
    if (loaded && typeof loaded === "object") settings = { ...DEFAULT_SETTINGS, ...loaded };
  } catch (_) {
    settings = { ...DEFAULT_SETTINGS };
  }

  await loadLanguage(settings.language);
  applyStyle();
  applyTranslations();
  initTabs();
  initHome();
  initGames();
  initInventory();
  initSettings();

  try {
    const version = await invoke("app_version");
    if (version) $("#app-version").textContent = `v${version}`;
  } catch (_) {
    $("#app-version").textContent = "";
  }

  try { state = await invoke("get_state"); } catch (_) { state = null; }
  selectTab(recall("kdf-tab") || "home");

  await listen("farm-state", (event) => {
    state = event.payload;
    if (state && typeof state.paused === "boolean") settings.paused = state.paused;
    render();
  });

  window.addEventListener("resize", debounce(() => {
    delete rendered.graph;
    if (state?.session) drawGraph(state.session);
  }, 120));

  if (!TAURI) {
    window.__preview = (next, patch) => {
      if (patch) {
        Object.assign(settings, patch);
        applyStyle();
        syncSettingsInputs();
      }
      state = next;
      rerenderAll();
    };
  }

  invoke("ui_ready").catch(() => {});
}

main();
