import {
  APPS,
  MODULES,
  PRESETS,
  SETTINGS,
  appsByCategory,
  emptyConfig,
} from "./catalog.js";
import {
  brewfilePreview,
  copyText,
  downloadText,
  generateConfigJson,
  generateSetupScript,
} from "./generator.js";

const state = emptyConfig();

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

function toggleInList(list, id, on) {
  const set = new Set(list);
  if (on) set.add(id);
  else set.delete(id);
  return [...set];
}

function renderPresets() {
  const root = $("#presets");
  root.innerHTML = PRESETS.map(
    (p) => `
    <button type="button" class="preset" data-preset="${p.id}">
      <span class="preset-label">${p.label}</span>
      <span class="preset-desc">${p.description}</span>
    </button>`
  ).join("");

  root.addEventListener("click", (e) => {
    const btn = e.target.closest("[data-preset]");
    if (!btn) return;
    const preset = PRESETS.find((p) => p.id === btn.dataset.preset);
    if (!preset) return;
    state.apps = [...preset.apps];
    state.settings = [...preset.settings];
    state.modules = [...preset.modules];
    syncUI();
    flash(`Applied ${preset.label} preset`);
  });
}

function checkbox(id, name, description, checked, group) {
  return `
    <label class="check">
      <input type="checkbox" data-group="${group}" data-id="${id}" ${checked ? "checked" : ""} />
      <span class="check-body">
        <span class="check-name">${name}</span>
        <span class="check-desc">${description}</span>
      </span>
    </label>`;
}

function renderApps() {
  const root = $("#apps");
  const byCat = appsByCategory();
  let html = "";
  for (const [category, apps] of byCat) {
    html += `<div class="group">
      <h3 class="group-title">${category}</h3>
      <div class="check-grid">
        ${apps
          .map((a) =>
            checkbox(a.id, a.name, a.description, state.apps.includes(a.id), "apps")
          )
          .join("")}
      </div>
    </div>`;
  }
  root.innerHTML = html;
}

function renderSettings() {
  $("#settings").innerHTML = `
    <div class="check-grid">
      ${SETTINGS.map((s) =>
        checkbox(
          s.id,
          s.name,
          s.description,
          state.settings.includes(s.id),
          "settings"
        )
      ).join("")}
    </div>`;
}

function renderModules() {
  $("#modules").innerHTML = `
    <div class="check-grid">
      ${MODULES.map((m) =>
        checkbox(
          m.id,
          m.name,
          m.description,
          state.modules.includes(m.id),
          "modules"
        )
      ).join("")}
    </div>`;
}

function bindChecks(container) {
  container.addEventListener("change", (e) => {
    const input = e.target;
    if (!(input instanceof HTMLInputElement) || input.type !== "checkbox") return;
    const group = input.dataset.group;
    const id = input.dataset.id;
    if (!group || !id) return;
    state[group] = toggleInList(state[group], id, input.checked);
    updateSummary();
  });
}

function syncIdentityFields() {
  $("#git-name").value = state.git.name || "";
  $("#git-email").value = state.git.email || "";
  $("#dotfiles-repo").value = state.dotfiles.repo || "";
}

function readIdentityFields() {
  state.git.name = $("#git-name").value.trim();
  state.git.email = $("#git-email").value.trim();
  state.dotfiles.repo = $("#dotfiles-repo").value.trim();
}

function syncUI() {
  renderApps();
  renderSettings();
  renderModules();
  syncIdentityFields();
  updateSummary();
}

function updateSummary() {
  readIdentityFields();
  $("#count-apps").textContent = String(state.apps.length);
  $("#count-settings").textContent = String(state.settings.length);
  $("#count-modules").textContent = String(state.modules.length);
  $("#brewfile-preview").textContent = brewfilePreview(state.apps);
}

function currentConfig() {
  readIdentityFields();
  return structuredClone(state);
}

function flash(message) {
  const el = $("#toast");
  el.textContent = message;
  el.dataset.show = "true";
  clearTimeout(flash._t);
  flash._t = setTimeout(() => {
    el.dataset.show = "false";
  }, 2200);
}

function applyImported(config) {
  if (!config || typeof config !== "object") throw new Error("Invalid JSON");
  state.git = {
    name: config.git?.name || "",
    email: config.git?.email || "",
  };
  state.dotfiles = { repo: config.dotfiles?.repo || "" };
  state.apps = Array.isArray(config.apps) ? config.apps.filter((id) => APPS.some((a) => a.id === id)) : [];
  state.settings = Array.isArray(config.settings) ? [...config.settings] : [];
  state.modules = Array.isArray(config.modules) ? [...config.modules] : [];
  syncUI();
}

function init() {
  renderPresets();
  syncUI();
  bindChecks($("#apps"));
  bindChecks($("#settings"));
  bindChecks($("#modules"));

  ["git-name", "git-email", "dotfiles-repo"].forEach((id) => {
    $(`#${id}`).addEventListener("input", updateSummary);
  });

  $("#btn-download-script").addEventListener("click", () => {
    const script = generateSetupScript(currentConfig());
    downloadText(script, "macbook-setup.sh", "text/x-shellscript");
    flash("Downloaded setup.sh");
  });

  $("#btn-copy-script").addEventListener("click", async () => {
    const script = generateSetupScript(currentConfig());
    await copyText(script);
    flash("Script copied to clipboard");
  });

  $("#btn-export-json").addEventListener("click", () => {
    downloadText(
      generateConfigJson(currentConfig()),
      "macbook-setup.machine.json",
      "application/json"
    );
    flash("Exported JSON backup");
  });

  $("#btn-copy-json").addEventListener("click", async () => {
    await copyText(generateConfigJson(currentConfig()));
    flash("JSON copied");
  });

  $("#btn-import-json").addEventListener("click", () => $("#import-file").click());

  $("#import-file").addEventListener("change", async (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const text = await file.text();
      applyImported(JSON.parse(text));
      flash(`Imported ${file.name}`);
    } catch (err) {
      flash("Could not import JSON");
      console.error(err);
    }
    e.target.value = "";
  });

  $("#btn-clear").addEventListener("click", () => {
    Object.assign(state, emptyConfig());
    syncUI();
    flash("Cleared selections");
  });

  // Reveal sections with a light stagger
  requestAnimationFrame(() => {
    document.body.dataset.ready = "true";
  });
}

init();
