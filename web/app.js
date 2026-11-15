// Airzone gateway installer page. Everything shown comes from the catalogue (/api/v1/catalogue) and the
// live values (/api/v1/systems/...). All UI text is in STRINGS so it can be translated in one place.
"use strict";

const STRINGS = {
  loading: "Loading…",
  gateway: "Gateway", system: "System", zones: "Zones", iaq: "Air quality sensor",
  information: "Information", configuration: "Configuration",
  login: "Log in", logout: "Log out", loggedIn: "Admin", readOnly: "Read only",
  setPassword: "Set the admin password", setPasswordHelp: "No admin password yet. Choose one (8 characters or more); it protects every change.",
  password: "Admin password", cancel: "Cancel", wrongPassword: "Wrong password.", locked: "Too many attempts, wait a minute.",
  listenMode: "listen", masterMode: "master",
  listenNote: "Listen mode: the gateway only reads the bus. Settings can be changed once it runs in master mode.",
  apply: "Apply", advanced: "Advanced (installer only)", confirmType: "Type {c} to confirm",
  applied: "Applied.", rejected: "Refused by the central (code {c}).", noAnswer: "No answer from the central.",
  unverified: "Sent, but the new value could not be read back.", queued: "Sent…",
  devBanner: "Developer mode is ON: raw bus access is enabled.",
  devMode: "Developer mode", on: "On", off: "Off",
  journal: "Change journal", undo: "Undo", noChanges: "No change recorded yet.",
  tokens: "API tokens", newToken: "New token name", create: "Create", revoke: "Revoke",
  tokenShown: "Copy this token now, it is shown only once:", dismiss: "Done",
  network: "Network and MQTT", save: "Save", saved: "Saved. Restart the gateway to apply.",
  restart: "Restart", rediscover: "Rediscover installation", diagnostics: "Download diagnostics",
  backupTitle: "Installation backup", backup: "Download backup", restore: "Restore from file", confirmWords: "confirmation words (optional)",
  restoreResult: "Restore: {q} change(s) sent, {s} skipped.", restoreConfirm: "Dangerous settings in the file need their confirmation words (comma separated), or leave empty to skip them:",
  mode: "Bus mode", switchMaster: "Switch to master mode", switchListen: "Switch to listen mode",
  updateAvailable: "Firmware {v} is available", download: "download",
  install: "Install", installConfirm: "Install firmware {v}? The gateway downloads it, checks it and restarts; this takes a few minutes. Do not unplug it.",
  installNow: "Install now", installing: "Installing firmware {v}…", downloadingPct: "Downloading… {p}%", installingStep: "Installing…",
  restarting: "The gateway is restarting…", updateFailed: "Update failed: {r}", retry: "Retry", installRefused: "Install refused: {e}",
  checkUpdates: "Check for updates from the official releases",
  checkUpdatesHelp: "When on, the gateway contacts letherman255.github.io once a day to look for a new firmware. Nothing is installed without your confirmation. The field below stays free for a custom manifest URL.",
  savedNow: "Saved.",
  masterWarning: "Master mode makes the gateway the bus master. Unplug the Airzone webserver first: two masters on the bus corrupt each other's frames.",
  notBuilt: "This firmware is built without master mode.",
  zone: "Zone", master: "master zone", outlets: "Air outlets", none: "—", unknown: "unknown",
  changePassword: "Change password", oldPassword: "Current password", newPassword: "New password",
};
const t = (k, v = {}) => (STRINGS[k] || k).replace(/\{(\w+)\}/g, (_, x) => v[x] ?? "");

const OFFICIAL_MANIFEST = "https://letherman255.github.io/Open-Airzone-Gateway/ota.json";
const state = { confirmInstall: false, polling: false, token: sessionStorage.getItem("azgw_token"), info: null, catalogue: [], systems: [], view: "gateway", zone: null, editing: false, open: new Set() };
const $ = (s) => document.querySelector(s);
const el = (tag, attrs = {}, ...kids) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v; else if (k.startsWith("on")) e.addEventListener(k.slice(2), v); else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
  }
  for (const k of kids.flat()) e.append(k instanceof Node ? k : document.createTextNode(k ?? ""));
  return e;
};

async function api(method, path, body) {
  const h = { "Content-Type": "application/json" };
  if (state.token) h.Authorization = "Bearer " + state.token;
  const r = await fetch("/api/v1/" + path, { method, headers: h, body: body === undefined ? undefined : JSON.stringify(body) });
  let data = null;
  try { data = await r.json(); } catch { data = null; }
  if (r.status === 401 && state.token) { state.token = null; sessionStorage.removeItem("azgw_token"); renderHeader(); }
  return { status: r.status, data };
}

function toast(msg) {
  const e = $("#toast"); e.textContent = msg; e.hidden = false;
  clearTimeout(toast.t); toast.t = setTimeout(() => (e.hidden = true), 4000);
}

const canWrite = () => state.token && state.info && state.info.mode === "master";
const keyOf = (p) => p.id.split(".").slice(1).join(".");

function fmt(p, v) {
  if (v === null || v === undefined) return t("unknown");
  if (typeof v === "boolean") return v ? t("on") : t("off");
  if (typeof v === "number") return (Math.round(v * 100) / 100) + (p.unit ? " " + p.unit : "");
  return String(v);
}

function inputFor(p, v) {
  if (p.kind === "bool") return el("select", {}, el("option", { value: "true", selected: v === true }, t("on")), el("option", { value: "false", selected: v === false }, t("off")));
  if (p.kind === "enum") return el("select", {}, Object.values(p.values).map((n) => el("option", { value: n, selected: v === n }, n)));
  if (p.kind === "text") return el("input", { value: v ?? "", maxlength: 2 * p.length });
  if (p.kind === "command") return null;
  return el("input", { type: "number", value: v ?? "", step: p.scale, min: p.min ?? false, max: p.max ?? false });
}

function readInput(p, input) {
  if (p.kind === "bool") return input.value === "true";
  if (p.kind === "number") return Number(input.value);
  if (p.kind === "command") return true;
  return input.value;
}

async function waitWrite(id) {
  for (let i = 0; i < 20; i++) {
    await new Promise((r) => setTimeout(r, 1000));
    const { data } = await api("GET", "writes/" + id);
    if (!data || data.state === "queued") continue;
    if (data.state === "applied") toast(t("applied"));
    else if (data.state === "rejected") toast(t("rejected", { c: data.code }));
    else if (data.state === "no_answer") toast(t("noAnswer"));
    else toast(t("unverified"));
    refresh();
    return;
  }
}

async function write(p, target, value, confirm) {
  const body = Object.assign({ value }, target);
  if (confirm) body.confirm = confirm;
  const { status, data } = await api("POST", "params/" + p.id, body);
  if (status === 202) { toast(t("queued")); waitWrite(data.queued); }
  else toast((data && (data.warning || data.error)) || "HTTP " + status);
}

function paramRow(p, values, target) {
  const v = values ? values[keyOf(p)] : undefined;
  const tr = el("tr", {}, el("th", {}, p.ui.label || p.id), el("td", { class: "val" }, fmt(p, v)));
  const editable = p.writable && p.confidence === "confirmed" && canWrite();
  const cell = el("td", { class: "edit" });
  if (editable) {
    const input = inputFor(p, v);
    if (input) { input.addEventListener("focus", () => (state.editing = true)); input.addEventListener("blur", () => (state.editing = false)); }
    if (p.danger) {
      const conf = el("input", { placeholder: t("confirmType", { c: p.danger.confirm }), "aria-label": t("confirmType", { c: p.danger.confirm }) });
      const btn = el("button", { class: "danger", type: "button", disabled: true }, t("apply"));
      conf.addEventListener("input", () => (btn.disabled = conf.value !== p.danger.confirm));
      conf.addEventListener("focus", () => (state.editing = true)); conf.addEventListener("blur", () => (state.editing = false));
      btn.addEventListener("click", () => write(p, target, input ? readInput(p, input) : true, conf.value));
      cell.append(el("div", { class: "warning" }, p.danger.warning), el("div", { class: "row" }, input || "", conf, btn));
    } else {
      cell.append(el("div", { class: "row" }, input || "", el("button", { class: "primary", type: "button", onclick: () => write(p, target, input ? readInput(p, input) : true) }, t("apply"))));
    }
  }
  tr.append(cell);
  return tr;
}

function sectioned(params, values, target) {
  const frag = document.createDocumentFragment();
  const normal = params.filter((p) => !p.danger), risky = params.filter((p) => p.danger);
  const bySection = {};
  for (const p of normal) (bySection[p.ui.section || ""] ||= []).push(p);
  for (const [sec, ps] of Object.entries(bySection)) {
    if (sec) frag.append(el("h3", {}, sec));
    frag.append(el("table", {}, ps.sort((a, b) => a.ui.order - b.ui.order).map((p) => paramRow(p, values, target))));
  }
  if (risky.length) {
    // remember which Advanced sections are open, so live refreshes do not close them
    const key = state.view + "/" + (state.zone || "") + "/" + JSON.stringify(target);
    const d = el("details", { class: "advanced", open: state.open.has(key) }, el("summary", {}, t("advanced")),
      el("table", {}, risky.sort((a, b) => a.ui.order - b.ui.order).map((p) => paramRow(p, values, target))));
    d.addEventListener("toggle", () => (d.open ? state.open.add(key) : state.open.delete(key)));
    frag.append(d);
  }
  return frag;
}

const paramsFor = (screen, scope) => state.catalogue.filter((p) => p.ui.screen === screen && (!scope || p.scope === scope) && p.confidence !== "hypothesis");

async function viewSystem(addr) {
  const { data } = await api("GET", "systems/" + addr);
  if (!data) return el("p", { class: "error" }, "?");
  const tgt = { system: addr };
  const sysValues = Object.assign({}, data.values);
  return el("div", {},
    el("h2", {}, t("system") + " " + addr + " — " + t("information")),
    sectioned(paramsFor("system.info", "system"), sysValues, tgt),
    el("h2", {}, t("system") + " " + addr + " — " + t("configuration")),
    state.info.mode !== "master" ? el("p", { class: "muted" }, t("listenNote")) : "",
    sectioned(paramsFor("system.config", "system"), sysValues, tgt));
}

async function viewZones(addr) {
  const sys = state.systems.find((s) => s.addr === addr);
  if (state.zone) return viewZone(addr, state.zone);
  const cards = await Promise.all(sys.zones.map(async (z) => {
    const { data } = await api("GET", `systems/${addr}/zones/${z.n}`);
    const v = (data && data.values) || {};
    return el("div", { class: "zone" + (v.power === false ? " off" : ""), onclick: () => { state.zone = z.n; render(); }, tabindex: 0 },
      el("div", { class: "muted" }, t("zone") + " " + z.n + (z.master ? " · " + t("master") : "")),
      el("div", {}, el("b", {}, z.name)),
      el("div", { class: "t" }, v.temperature == null ? t("none") : v.temperature + " °C"),
      el("div", { class: "muted" }, (v.power ? (v.mode || "") : t("off")) + (v.setpoint != null ? " · " + v.setpoint + " °C" : "")));
  }));
  return el("div", {}, el("h2", {}, t("zones")), el("div", { class: "zones" }, cards));
}

async function viewZone(addr, n) {
  const { data } = await api("GET", `systems/${addr}/zones/${n}`);
  const sysd = (await api("GET", "systems/" + addr)).data;
  const tgt = { system: addr, zone: n };
  const outlets = (sysd ? sysd.outlets : []).filter((o) => o.values.zone === n);
  const outletParams = paramsFor("zones", "outlet");
  return el("div", {},
    el("button", { type: "button", onclick: () => { state.zone = null; render(); } }, "← " + t("zones")),
    el("h2", {}, (data ? data.name : "") + " (" + t("zone") + " " + n + ")"),
    sectioned(paramsFor("zones", "zone"), data && data.values, tgt),
    outlets.length ? el("h2", {}, t("outlets")) : "",
    outlets.map((o) => el("div", {}, el("h3", {}, "#" + o.n), sectioned(outletParams, o.values, { system: addr, outlet: o.n }))));
}

async function viewIaq(addr) {
  const { data } = await api("GET", "systems/" + addr);
  return el("div", {}, el("h2", {}, t("iaq")), sectioned(paramsFor("iaq", "iaq"), data && data.iaq, { system: addr }));
}

const updateRunning = (u) => !!u && !u.startsWith("failed");

// The update banner: available / confirm / progress / restarting / failed. Rebuilt in place while polling.
function updateBox() {
  const i = state.info, u = i.update_state || "";
  const box = el("div", { class: "warning", id: "updatebox" });
  if (u.startsWith("failed")) {
    box.append(el("b", {}, t("updateFailed", { r: u.replace(/^failed:?\s*/, "") })), " ");
    if (state.token && i.latest_url) box.append(el("button", { type: "button", onclick: () => { state.confirmInstall = true; refreshUpdateBox(); } }, t("retry")));
    return box;
  }
  if (u) {
    const m = /^downloading (\d+)%/.exec(u);
    box.append(t("installing", { v: i.latest_version || "" }), " ", el("b", {}, m ? t("downloadingPct", { p: m[1] }) : u === "restarting" ? t("restarting") : t("installingStep")));
    return box;
  }
  if (!i.latest_version || !i.latest_url) return null;
  box.append(t("updateAvailable", { v: i.latest_version }), " ");
  if (state.token && state.confirmInstall) {
    box.append(el("div", {}, t("installConfirm", { v: i.latest_version })), el("div", { class: "row" },
      el("button", { class: "danger", type: "button", onclick: startInstall }, t("installNow")),
      el("button", { type: "button", onclick: () => { state.confirmInstall = false; refreshUpdateBox(); } }, t("cancel"))));
    return box;
  }
  if (state.token) box.append(el("button", { class: "primary", type: "button", onclick: () => { state.confirmInstall = true; refreshUpdateBox(); } }, t("install")), " ");
  box.append(el("a", { href: i.latest_url, rel: "noopener" }, t("download")));
  return box;
}

function refreshUpdateBox() {
  const old = document.getElementById("updatebox"), nb = updateBox();
  if (old && nb) old.replaceWith(nb); else if (old) old.remove(); else if (nb && state.view === "gateway") $("#main").prepend(nb);
}

async function startInstall() {
  state.confirmInstall = false;
  const r = await api("POST", "update/install");
  if (r.status !== 202) { toast(t("installRefused", { e: (r.data && r.data.error) || "HTTP " + r.status })); refreshUpdateBox(); return; }
  state.info.update_state = "downloading 0%";
  refreshUpdateBox();
  pollUpdate();
}

// Poll /info every 2 s while an install runs; after the restart, reload once the gateway answers again.
async function pollUpdate() {
  if (state.polling) return;
  state.polling = true;
  let restarted = false, idle = 0;
  try {
    for (;;) {
      await new Promise((r) => setTimeout(r, 2000));
      let info = null;
      try { info = (await api("GET", "info")).data; } catch { if (restarted) continue; }
      if (!info) { if (restarted) continue; else break; }
      const u = info.update_state || "";
      if (restarted && u !== "restarting") { location.reload(); return; }
      state.info = info;
      if (u === "restarting") restarted = true;
      refreshUpdateBox();
      if (u.startsWith("failed")) break;
      if (!u) { if (!info.latest_version) { location.reload(); return; } if (++idle >= 5) break; }
    }
  } finally { state.polling = false; }
}

async function viewGateway() {
  const i = state.info;
  const rows = [["Firmware", `${i.version} (build ${i.build})`], ["Board", i.board], ["MAC", i.mac], ["IP", i.ip], [t("mode"), i.mode],
    ["MQTT", i.mqtt_configured ? (i.mqtt_connected ? "connected" : "not connected") : "not configured"], ["Uptime", i.uptime_s + " s"], ["Time", i.time || t("none")]];
  const out = el("div", {}, el("h2", {}, t("gateway")), el("table", {}, rows.map(([k, v]) => el("tr", {}, el("th", {}, k), el("td", {}, v)))));
  const ub = updateBox();
  if (ub) out.prepend(ub);
  if (updateRunning(i.update_state)) pollUpdate();
  if (!state.token) return out;
  // mode
  const modeBox = el("div", {}, el("h2", {}, t("mode")));
  if (!i.master_capable) modeBox.append(el("p", { class: "muted" }, t("notBuilt")));
  else if (i.configured_mode === "master") modeBox.append(el("button", { type: "button", onclick: async () => { await api("POST", "mode", { mode: "listen" }); toast(t("saved")); } }, t("switchListen")));
  else {
    const conf = el("input", { placeholder: t("confirmType", { c: "MASTER" }) });
    const b = el("button", { class: "danger", type: "button", disabled: true, onclick: async () => { await api("POST", "mode", { mode: "master" }); toast(t("saved")); } }, t("switchMaster"));
    conf.addEventListener("input", () => (b.disabled = conf.value !== "MASTER"));
    modeBox.append(el("div", { class: "warning" }, t("masterWarning")), el("div", { class: "row" }, conf, b));
  }
  out.append(modeBox);
  // developer mode
  out.append(el("h2", {}, t("devMode")), el("div", { class: "row" },
    el("button", { type: "button", onclick: async () => { await api("POST", "dev", { on: !i.dev_mode }); await loadInfo(); render(); } }, i.dev_mode ? t("off") : t("on"))));
  // journal
  const j = (await api("GET", "journal")).data || [];
  out.append(el("h2", {}, t("journal")), j.length ? el("table", {}, j.slice().reverse().map((e) => el("tr", {},
    el("td", {}, new Date(Date.now() - (i.uptime_s * 1000 - e.t_ms)).toLocaleString()), el("td", {}, e.id), el("td", { class: "val" }, `${e.old} → ${e.new}`),
    el("td", { class: "edit" }, el("button", { type: "button", disabled: i.mode !== "master", onclick: async () => { const r = await api("POST", `journal/${e.index}/undo`); if (r.status === 202) waitWrite(r.data.queued); else toast(r.data.error); } }, t("undo")))))) : el("p", { class: "muted" }, t("noChanges")));
  // tokens
  const toks = (await api("GET", "tokens")).data || [];
  const tn = el("input", { placeholder: t("newToken") });
  out.append(el("h2", {}, t("tokens")), el("table", {}, toks.map((n) => el("tr", {}, el("td", {}, n), el("td", { class: "edit" },
    el("button", { type: "button", onclick: async () => { await api("DELETE", "tokens/" + encodeURIComponent(n)); render(); } }, t("revoke")))))),
    el("div", { class: "row" }, tn, el("button", { type: "button", onclick: async () => { const r = await api("POST", "tokens", { name: tn.value }); if (r.status === 201) { state.newToken = r.data.token; render(); } else toast(r.data.error); } }, t("create"))));
  // a new token stays under the form (live refreshes re-render the page) until dismissed
  if (state.newToken) {
    out.append(el("div", { class: "warning" }, t("tokenShown"), " ", el("code", { style: "user-select:all" }, state.newToken), " ",
      el("button", { type: "button", onclick: () => { state.newToken = null; render(); } }, t("dismiss"))));
  }
  // network
  const s = (await api("GET", "settings")).data || {};
  const fields = ["wifi_ssid", "wifi_pass", "mqtt_url", "mqtt_user", "mqtt_pass", "mqtt_prefix", "gateway_name", "timezone", "update_manifest"];
  const inputs = Object.fromEntries(fields.map((f) => [f, el("input", { value: f.endsWith("pass") ? "" : (s[f] || ""), type: f.endsWith("pass") ? "password" : "text", placeholder: f.endsWith("pass") && s[f] ? "•••• (unchanged)" : "" })]));
  const check = el("input", { type: "checkbox", id: "checkupdates", checked: s.update_manifest === OFFICIAL_MANIFEST });
  check.addEventListener("change", async () => {
    const v = check.checked ? OFFICIAL_MANIFEST : "";
    inputs.update_manifest.value = v;
    const r = await api("POST", "settings", { update_manifest: v });
    toast(r.status === 200 ? t("savedNow") : r.data.error);
  });
  inputs.update_manifest.addEventListener("input", () => (check.checked = inputs.update_manifest.value === OFFICIAL_MANIFEST));
  out.append(el("h2", {}, t("network")),
    el("div", { class: "row" }, el("label", {}, check, " ", t("checkUpdates"))), el("p", { class: "muted" }, t("checkUpdatesHelp")),
    el("table", {}, fields.map((f) => el("tr", {}, el("th", {}, f), el("td", {}, inputs[f])))),
    el("div", { class: "row" }, el("button", { class: "primary", type: "button", onclick: async () => {
      const body = {}; for (const f of fields) if (!(f.endsWith("pass") && !inputs[f].value)) body[f] = inputs[f].value;
      const r = await api("POST", "settings", body); toast(r.status === 200 ? t("saved") : r.data.error);
    } }, t("save")), el("button", { type: "button", onclick: () => api("POST", "restart") }, t("restart")),
    el("button", { type: "button", onclick: () => api("POST", "rediscover") }, t("rediscover")),
    el("button", { type: "button", onclick: async () => { const r = await api("GET", "diagnostics"); const a = el("a", { href: URL.createObjectURL(new Blob([JSON.stringify(r.data, null, 1)], { type: "application/json" })), download: "airzone-gateway-diagnostics.json" }); a.click(); } }, t("diagnostics"))));
  // backup / restore
  const file = el("input", { type: "file", accept: "application/json", hidden: true });
  const conf = el("input", { placeholder: t("confirmWords"), "aria-label": t("restoreConfirm") });
  file.addEventListener("change", async () => {
    const f = file.files[0]; if (!f) return;
    let body; try { body = JSON.parse(await f.text()); } catch { toast("JSON?"); return; }
    body.confirm = conf.value.split(",").map((x) => x.trim()).filter(Boolean);
    const r = await api("POST", "restore", body);
    if (r.status !== 200) { toast(r.data.error); return; }
    toast(t("restoreResult", { q: r.data.queued.length, s: r.data.skipped.length }));
    if (r.data.skipped.length) out.append(el("div", { class: "warning" }, r.data.skipped.map((x) => x.id + ": " + x.reason).join(" · ")));
    r.data.queued.forEach((id) => waitWrite(id));
  });
  out.append(el("h2", {}, t("backupTitle")), el("div", { class: "row" },
    el("button", { type: "button", onclick: async () => { const r = await api("GET", "backup"); const a = el("a", { href: URL.createObjectURL(new Blob([JSON.stringify(r.data, null, 1)], { type: "application/json" })), download: "airzone-installation-backup.json" }); a.click(); } }, t("backup")),
    el("button", { type: "button", disabled: i.mode !== "master", onclick: () => file.click() }, t("restore")), file),
    el("p", { class: "muted" }, t("restoreConfirm")), conf);
  return out;
}

function renderHeader() {
  const i = state.info || {};
  $("#gwname").textContent = i.name || "Airzone gateway";
  const b = $("#modebadge"); b.textContent = i.mode === "master" ? t("masterMode") : t("listenMode"); b.className = "badge " + (i.mode || "");
  $("#authstate").textContent = state.token ? t("loggedIn") : t("readOnly");
  $("#loginbtn").textContent = state.token ? t("logout") : t("login");
  const d = $("#devbanner"); d.hidden = !i.dev_mode; d.textContent = t("devBanner");
}

function renderNav() {
  const nav = $("#nav"); nav.replaceChildren();
  const tabs = [["gateway", t("gateway")]];
  for (const s of state.systems) {
    tabs.push(["system:" + s.addr, t("system") + " " + s.addr], ["zones:" + s.addr, t("zones")]);
    if (s.iaq) tabs.push(["iaq:" + s.addr, t("iaq")]);
  }
  for (const [k, label] of tabs) nav.append(el("button", { type: "button", class: state.view === k ? "active" : "", onclick: () => { state.view = k; state.zone = null; render(); } }, label));
}

async function render() {
  renderHeader(); renderNav();
  const [kind, addr] = state.view.split(":");
  const a = Number(addr);
  const view = kind === "system" ? await viewSystem(a) : kind === "zones" ? await viewZones(a) : kind === "iaq" ? await viewIaq(a) : await viewGateway();
  const y = window.scrollY;
  $("#main").replaceChildren(view);
  window.scrollTo(0, y);
}

async function refresh() {
  if (state.editing || document.activeElement && ["INPUT", "SELECT"].includes(document.activeElement.tagName)) return;
  await loadInfo();
  render();
}

async function loadInfo() {
  state.info = (await api("GET", "info")).data;
  state.systems = (await api("GET", "systems")).data || [];
}

async function login() {
  const dlg = $("#logindlg");
  const first = !state.info.password_set;
  $("#logintitle").textContent = first ? t("setPassword") : t("login");
  $("#loginhelp").textContent = first ? t("setPasswordHelp") : "";
  $("#pwlabel").textContent = t("password");
  $("#loginsubmit").textContent = first ? t("save") : t("login");
  $("#logincancel").textContent = t("cancel");
  $("#loginerror").textContent = "";
  $("#pw").value = "";
  dlg.showModal();
}

$("#logincancel").addEventListener("click", () => $("#logindlg").close());
$("#loginform").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const pw = $("#pw").value;
  if (!state.info.password_set) {
    const r = await api("POST", "password", { new: pw });
    if (r.status !== 200) { $("#loginerror").textContent = r.data.error; return; }
  }
  const r = await api("POST", "login", { password: pw });
  if (r.status === 200) {
    state.token = r.data.token; sessionStorage.setItem("azgw_token", state.token);
    $("#logindlg").close(); await loadInfo(); render();
  } else $("#loginerror").textContent = r.status === 423 ? t("locked") : t("wrongPassword");
});
$("#loginbtn").addEventListener("click", async () => {
  if (state.token) { await api("POST", "logout"); state.token = null; sessionStorage.removeItem("azgw_token"); await loadInfo(); render(); }
  else login();
});

function live() {
  try {
    const ws = new WebSocket((location.protocol === "https:" ? "wss://" : "ws://") + location.host + "/api/v1/ws");
    let last = 0;
    // the gateway registers a client on its first frame (ESP-IDF does not report the handshake)
    ws.onopen = () => ws.send("hello");
    ws.onmessage = () => { if (Date.now() - last > 4000) { last = Date.now(); refresh(); } };
    ws.onclose = () => setTimeout(live, 5000);
  } catch { setTimeout(live, 5000); }
}

(async function start() {
  $("#loading").textContent = t("loading");
  state.catalogue = ((await api("GET", "catalogue")).data || {}).params || [];
  await loadInfo();
  await render();
  live();
})();
