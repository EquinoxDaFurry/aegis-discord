const pages = [...document.querySelectorAll(".page")];
const navLinks = [...document.querySelectorAll("[data-page]")];
const breadcrumb = document.getElementById("breadcrumb-current");
const toast = document.getElementById("toast");
let toastTimer;
function showToast(message){toast.textContent=message;toast.classList.add("show");clearTimeout(toastTimer);toastTimer=setTimeout(()=>toast.classList.remove("show"),3400)}
function navigate(page){const target=document.getElementById("page-"+page);if(!target)return;pages.forEach(p=>p.classList.toggle("active",p===target));navLinks.forEach(link=>link.classList.toggle("active",link.dataset.page===page));const selectorMode=page==="selector";document.body.classList.toggle("selector-mode",selectorMode);breadcrumb.textContent=selectorMode?"Select a server":page==="owner"?"Owner console":page==="account"?"My account":page.charAt(0).toUpperCase()+page.slice(1);history.replaceState(null,"",selectorMode?"#select-server":"#"+page);document.getElementById("sidebar").classList.remove("open");window.scrollTo({top:0,behavior:"smooth"});if(page==="moderation")window.dispatchEvent(new Event("aegis:moderation-open"));}
navLinks.forEach(link=>link.addEventListener("click",event=>{event.preventDefault();navigate(link.dataset.page)}));
document.querySelectorAll("[data-page-link]").forEach(button=>button.addEventListener("click",()=>navigate(button.dataset.pageLink)));
document.getElementById("mobile-menu").addEventListener("click",()=>document.getElementById("sidebar").classList.toggle("open"));
document.getElementById("server-picker").addEventListener("click",()=>navigate("selector"));
document.querySelector(".notice-close")?.addEventListener("click",event=>event.currentTarget.closest(".notice").remove());
document.getElementById("help-button").addEventListener("click",()=>showToast("Documentation link will be connected in a later build."));
const templates={
 timeout:{title:"Timeout notice",description:"Exact default DM sent to a member after Aegis detects scam content.",content:"Hello {user_id},\n\n⚠️ **Security Alert**\n\nYour account sent content commonly associated with scam campaigns.\n\nYour account may have been compromised.\n\nPlease:\n\n🔒 Reset your Discord password\n🛡️ Enable Two-Factor Authentication\n🦠 Run malware scans\n🔗 Review Authorized Apps\n\n\nYou have been timed out in:\n\n**{server}**\n\nfor **{duration} days**.\n\n\nDetected location:\n\n{channel}\n\n\nFlagged Content:\n\n\`\`\`text\n{flagged_content}\n\`\`\`\n\n\nIf this was a mistake:\n\n{report_server}\n\n\nPlease include:\n- The flagged message\n- What you intended to send\n- Any useful investigation details\n\n\n-# Anti-Scam Protection System"},
 detection:{title:"Detection alert",description:"Exact default audit-log embed posted in the configured logging channel.",content:"🚨 Scam Detected\nPossible scam content detected.\n\nUser\n{user}\\n\`{user_id}\`\n\nReason\n{reason}\n\nFlagged Content\n\`\`\`text\n{flagged_content}\n\`\`\`\n\nChannel\n{channel}\n\nDeleted\n{deleted}\n\nTimeout\n{timeout_status}\n\n[Button: ⚠ False Positive]"},
 report:{title:"False-positive confirmation",description:"Exact ephemeral confirmation shown after a report is successfully submitted.",content:"✅ False-positive report submitted."}
};
let currentTemplate="timeout";
const editor=document.getElementById("message-content");
function renderTemplate(key){currentTemplate=key;const t=templates[key];document.querySelectorAll(".template-choice").forEach(b=>b.classList.toggle("selected",b.dataset.template===key));document.getElementById("template-title").textContent=t.title;document.getElementById("template-description").textContent=t.description;editor.value=t.content;document.getElementById("template-enabled").checked=true;updatePreview();document.getElementById("message-status").textContent="Preview template loaded"}
let previewServerName = "Select a server";
function updatePreview(){const example={user:"@sample_member",user_id:"123456789012345678",server:previewServerName,reason:"Image attachment matched a known scam campaign.",duration:"7",channel:"<#123456789012345678>",flagged_content:"[Flagged message content]",report_server:"Not configured",deleted:"true",timeout_status:"Timed out successfully"};let value=editor.value;for(const [key,val] of Object.entries(example))value=value.replaceAll("{"+key+"}",val);document.getElementById("message-preview").textContent=value}
document.querySelectorAll(".template-choice").forEach(button=>button.addEventListener("click",()=>renderTemplate(button.dataset.template)));
editor.addEventListener("input",()=>{updatePreview();document.getElementById("message-status").textContent="Unsaved preview changes"});
document.getElementById("template-enabled").addEventListener("change",event=>{document.getElementById("message-status").textContent=event.target.checked?"Template enabled in preview":"Template disabled in preview"});
document.querySelectorAll("[data-placeholder]").forEach(button=>button.addEventListener("click",()=>{const start=editor.selectionStart,end=editor.selectionEnd,token=button.dataset.placeholder;editor.setRangeText(token,start,end,"end");editor.focus();updatePreview();document.getElementById("message-status").textContent="Unsaved preview changes"}));
document.getElementById("save-message").addEventListener("click",()=>{showToast("Preview only — the Rust API is not connected, so nothing has been saved.");document.getElementById("message-status").textContent="Not saved · backend not connected"});
const search=document.getElementById("case-search"),filter=document.getElementById("case-filter");
function filterCases(){const q=(search.value||"").toLowerCase();document.querySelectorAll("#case-rows tr").forEach(row=>{const matches=row.dataset.search.includes(q);const status=filter.value==="all"||row.dataset.status===filter.value;row.classList.toggle("hidden-row",!(matches&&status))})}
search.addEventListener("input",filterCases);filter.addEventListener("change",filterCases);
document.querySelectorAll("[data-case-action]").forEach(button=>button.addEventListener("click",()=>showToast("Case review is a UI preview. No timeout has been changed or reversed.")));
document.querySelectorAll(".nav-link").forEach(link=>link.addEventListener("click",()=>{}));
navigate("selector");

// Discord account, server selection, and per-server dashboard data.
(async function dashboardBackend() {
  const ownerCard = document.querySelector(".owner-card");
  if (ownerCard) ownerCard.remove();
  const ownerNav = document.querySelector('[data-page="owner"]');
  if (ownerNav) ownerNav.remove();

  let activeGuild = null;
  let signedIn = false;
  let currentSettings = null;
  let betaBotPresent = false;
  let betaStatusRequest = 0;
  let moderationData = [];
  let moderationRequest = 0;
  const settingsPage = document.getElementById("page-settings");

  function formatDate(value) {
    const date = new Date(Number(value) * 1000);
    return Number.isNaN(date.getTime()) ? "Unknown date" : date.toLocaleString();
  }
  function formatTimeoutUntil(value) {
    if (!value) return "Unknown";
    const date = /^\d+$/.test(value) ? new Date(Number(value) * 1000) : new Date(value);
    return Number.isNaN(date.getTime()) ? "Unknown" : date.toLocaleString();
  }
  function statusLabel(status) {
    return ({active:"ACTIVE",expired:"EXPIRED",ended_early:"ENDED EARLY",unknown:"UNVERIFIED"})[status] || "UNKNOWN";
  }
  function statusClass(status) {
    return status === "active" ? "active" : status === "expired" ? "expired" : status === "ended_early" ? "ended" : "unknown";
  }
  function renderModerationRows() {
    const rows = document.getElementById("moderation-rows");
    if (!rows) return;
    const query = (document.getElementById("moderation-search")?.value || "").toLowerCase();
    const filter = document.getElementById("moderation-filter")?.value || "all";
    const visible = moderationData.filter(item => {
      const matches = (item.username + " " + item.user_id + " " + item.reason).toLowerCase().includes(query);
      return matches && (filter === "all" || item.status === filter);
    });
    rows.replaceChildren();
    if (!visible.length) {
      const tr = document.createElement("tr"), td = document.createElement("td");
      td.colSpan = 6; td.className = "moderation-empty-cell";
      td.textContent = moderationData.length ? "No incidents match your search or status filter." : "No successful Aegis timeouts were found in the last 9 days.";
      tr.appendChild(td); rows.appendChild(tr);
    } else visible.forEach(item => {
      const tr = document.createElement("tr");
      const member = document.createElement("td");
      const memberWrap = document.createElement("div"); memberWrap.className = "moderation-member";
      const avatar = document.createElement("img"); avatar.className = "moderation-avatar";
      avatar.alt = ""; avatar.src = item.avatar_url || "https://cdn.discordapp.com/embed/avatars/0.png";
      avatar.addEventListener("error", () => { avatar.src = "https://cdn.discordapp.com/embed/avatars/0.png"; }, {once:true});
      const memberCopy = document.createElement("span");
      const username = document.createElement("b"); username.textContent = item.username;
      const userId = document.createElement("small"); userId.textContent = item.user_id;
      memberCopy.append(username,userId); memberWrap.append(avatar,memberCopy); member.appendChild(memberWrap);
      const reason = document.createElement("td"); reason.textContent = item.reason;
      const duration = document.createElement("td"); duration.textContent = item.timeout_days + " days";
      const detected = document.createElement("td"); detected.textContent = formatDate(item.created_at);
      const status = document.createElement("td"); const pill = document.createElement("span");
      pill.className = "moderation-status " + statusClass(item.status); pill.textContent = statusLabel(item.status); status.appendChild(pill);
      const action = document.createElement("td"); const button = document.createElement("button");
      button.className = "text-action"; button.type = "button"; button.textContent = "View details";
      button.addEventListener("click", () => showModerationDetail(item)); action.appendChild(button);
      tr.append(member,reason,duration,detected,status,action); rows.appendChild(tr);
    });
    const count = document.getElementById("moderation-count");
    if (count) count.textContent = "Showing " + visible.length + " of " + moderationData.length + " incidents";
  }
  function showModerationDetail(item) {
    const panel = document.getElementById("moderation-detail");
    const content = document.getElementById("moderation-detail-content");
    if (!panel || !content) return;
    document.getElementById("moderation-detail-subtitle").textContent = item.username + " · " + formatDate(item.created_at);
    content.replaceChildren();
    const grid = document.createElement("div"); grid.className = "moderation-detail-grid";
    const addField = (label, value, className="") => {
      const box = document.createElement("div"); box.className = "moderation-detail-field " + className;
      const name = document.createElement("small"); name.textContent = label;
      const val = document.createElement("div"); val.textContent = value || "Not available";
      box.append(name,val); grid.appendChild(box);
    };
    addField("MEMBER", item.username + " · " + item.user_id);
    addField("DETECTION REASON", item.reason);
    addField("TIMEOUT STATUS", statusLabel(item.status));
    addField("TIMEOUT DURATION", item.timeout_days + " days");
    addField("TIMEOUT ENDS", formatTimeoutUntil(item.timeout_until));
    addField("DETECTED AT", formatDate(item.created_at));
    addField("CONFIDENCE", item.confidence == null ? "Not included in log" : item.confidence.toFixed(2) + "%");
    addField("ORIGINAL CONFIDENCE", item.original_confidence == null ? "Not included in log" : item.original_confidence.toFixed(2) + "%");
    addField("MESSAGE DELETED", item.deleted ? "Yes" : "No");
    addField("CHANNEL", item.channel_id ? "<#" + item.channel_id + ">" : "Not recorded");
    content.appendChild(grid);
    const flagged = document.createElement("div"); flagged.className = "moderation-flagged";
    const label = document.createElement("small"); label.textContent = "FLAGGED CONTENT";
    const body = document.createElement("pre"); body.textContent = item.flagged_content;
    flagged.append(label,body); content.appendChild(flagged);
    const footer = document.createElement("div"); footer.className = "moderation-detail-actions";
    const link = document.createElement("a"); link.className = "button button-secondary"; link.href = item.log_url;
    link.target = "_blank"; link.rel = "noopener noreferrer"; link.textContent = "Open Discord log ↗";
    footer.appendChild(link); content.appendChild(footer);
    panel.hidden = false; panel.scrollIntoView({behavior:"smooth",block:"start"});
  }
  async function loadModeration() {
    if (!activeGuild) return;
    const guildId = activeGuild.id, requestId = ++moderationRequest;
    const rows = document.getElementById("moderation-rows");
    const warning = document.getElementById("moderation-warning");
    const detail = document.getElementById("moderation-detail");
    if (detail) detail.hidden = true;
    if (rows) rows.innerHTML = '<tr><td colspan="6" class="muted-cell">Loading moderation history from Discord…</td></tr>';
    if (warning) warning.hidden = true;
    try {
      const data = await api("/api/guilds/" + encodeURIComponent(guildId) + "/moderation");
      if (requestId !== moderationRequest || activeGuild?.id !== guildId) return;
      moderationData = data.incidents || [];
      document.getElementById("moderation-window").textContent = "LAST " + data.lookback_days + " DAYS";
      const counts = {active:0,expired:0,ended_early:0,unknown:0};
      moderationData.forEach(item => counts[item.status] = (counts[item.status] || 0) + 1);
      document.getElementById("moderation-total").textContent = moderationData.length;
      document.getElementById("moderation-active").textContent = counts.active;
      document.getElementById("moderation-expired").textContent = counts.expired;
      document.getElementById("moderation-ended").textContent = counts.ended_early;
      document.getElementById("moderation-source").textContent = data.logging_channel_id ? "Source: Discord logging channel" : "Logging channel not configured";
      if (data.warning) {
        warning.hidden = false;
        document.getElementById("moderation-warning-title").textContent = "Logging channel not found";
        document.getElementById("moderation-warning-text").textContent = data.warning;
      } else if (data.truncated) {
        warning.hidden = false;
        document.getElementById("moderation-warning-title").textContent = "History scan limit reached";
        document.getElementById("moderation-warning-text").textContent = "There are more than 10,000 recent log messages. The list may not include the full 9-day window.";
      }
      renderModerationRows();
    } catch (error) {
      if (requestId !== moderationRequest) return;
      moderationData = [];
      renderModerationRows();
      warning.hidden = false;
      document.getElementById("moderation-warning-title").textContent = "Could not load moderation history";
      document.getElementById("moderation-warning-text").textContent = error.message + ". Check that Aegis can view the logging channel and read its message history.";
      ["moderation-total","moderation-active","moderation-expired","moderation-ended"].forEach(id => document.getElementById(id).textContent = "—");
    }
  }
  window.addEventListener("aegis:moderation-open", loadModeration);
  document.getElementById("moderation-refresh")?.addEventListener("click", loadModeration);
  document.getElementById("moderation-search")?.addEventListener("input", renderModerationRows);
  document.getElementById("moderation-filter")?.addEventListener("change", renderModerationRows);
  document.getElementById("moderation-close-detail")?.addEventListener("click", () => { document.getElementById("moderation-detail").hidden = true; });
  const asset = name => "https://aegisdiscord.pages.dev/assets/" + name;

  async function api(path, options = {}) {
    const response = await fetch(path, { credentials: "same-origin", ...options,
      headers: { "Content-Type": "application/json", ...(options.headers || {}) } });
    if (response.status === 401) { signedIn = false; throw new Error("Please sign in with Discord again."); }
    if (!response.ok) throw new Error("Request failed (" + response.status + ")");
    return response.json();
  }

  function initials(name) {
    return (name || "?").trim().split(/\s+/).filter(Boolean).map(word => Array.from(word)[0]).join("").toUpperCase() || "?";
  }

  function serverIcon(guild, muted = false) {
    const fallback = document.createElement("span");
    fallback.className = "server-card-initials";
    fallback.textContent = initials(guild.name);
    if (!guild.icon) return fallback;
    const image = document.createElement("img");
    image.className = "server-card-image" + (muted ? " grayscale" : "");
    image.src = "https://cdn.discordapp.com/icons/" + encodeURIComponent(guild.id) + "/" + encodeURIComponent(guild.icon) + ".png?size=128";
    image.alt = "";
    image.loading = "lazy";
    image.addEventListener("error", () => image.replaceWith(fallback), { once: true });
    return image;
  }

  function serverCard(guild, manageable) {
    const card = document.createElement(manageable ? "button" : "div");
    card.className = "server-card" + (manageable ? "" : " server-card-muted");
    if (manageable) {
      card.type = "button";
      card.addEventListener("click", () => enterServer(guild));
      card.setAttribute("aria-label", "Manage " + guild.name);
    } else {
      card.setAttribute("aria-label", guild.name + ", view only");
      card.setAttribute("aria-disabled", "true");
    }
    const iconWrap = document.createElement("span");
    iconWrap.className = "server-card-icon";
    iconWrap.appendChild(serverIcon(guild, !manageable));
    card.appendChild(iconWrap);
    const copy = document.createElement("span");
    copy.className = "server-card-copy";
    const name = document.createElement("strong");
    name.textContent = guild.name;
    const detail = document.createElement("small");
    detail.textContent = manageable ? "Open server dashboard" : "You don't have management access";
    copy.append(name, detail);
    card.appendChild(copy);
    if (manageable) {
      const status = document.createElement("img");
      status.className = "server-access-icon";
      status.src = asset("chaching.svg");
      status.alt = "Manageable";
      card.appendChild(status);
      card.appendChild(Object.assign(document.createElement("span"), { className: "server-card-arrow", textContent: "→" }));
    } else {
      const restricted = document.createElement("span");
      restricted.className = "server-restricted";
      restricted.innerHTML = '<span aria-hidden="true">🔒</span><img src="' + asset("euheuh.svg") + '" alt="No management access">';
      card.appendChild(restricted);
    }
    return card;
  }

  function renderSelector(guilds) {
    const manageable = guilds.filter(g => g.owner || ((BigInt(g.permissions || "0") & 40n) !== 0n)).sort((a,b)=>a.name.localeCompare(b.name));
    const other = guilds.filter(g => !manageable.includes(g)).sort((a,b)=>a.name.localeCompare(b.name));
    const manageGrid = document.getElementById("server-grid-manageable");
    const otherGrid = document.getElementById("server-grid-other");
    manageGrid.replaceChildren(...manageable.map(g => serverCard(g, true)));
    otherGrid.replaceChildren(...other.map(g => serverCard(g, false)));
    document.getElementById("selector-manageable-section").hidden = manageable.length === 0;
    document.getElementById("selector-other-section").hidden = other.length === 0;
    document.getElementById("selector-empty").hidden = guilds.length !== 0;
    if (!manageable.length && guilds.length) {
      const message = document.getElementById("selector-empty");
      message.hidden = false;
      message.innerHTML = "<h2>No manageable servers</h2><p>Your servers are listed below, but none currently grant Administrator or Manage Server permission. Ask a server administrator to update your role if you need access.</p>";
    }
  }

  function settingsForm() {
    settingsPage.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">SERVER CONFIGURATION / SETTINGS</div><h1>Server settings</h1><p>Settings are saved separately for each Discord server.</p></div></div>
      <div class="panel" style="padding:24px;max-width:760px">
        <div id="settings-server-name" style="font-weight:700;margin-bottom:22px"></div>
        <form id="live-settings-form">
          <label class="field-label" style="display:block;margin-bottom:8px"><input id="setting-enabled" type="checkbox" checked> Enable Aegis for this server</label>
          <div style="margin-top:22px"><label class="field-label" for="setting-timeout">DEFAULT TIMEOUT DURATION (DAYS)</label>
            <input id="setting-timeout" type="number" min="1" max="7" step="1" value="7" style="display:block;width:100%;max-width:220px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="margin-top:22px"><label class="field-label" for="setting-threshold">IMAGE MATCH CONFIDENCE THRESHOLD</label>
            <input id="setting-threshold" type="number" min="0.5" max="1" step="0.005" value="0.805" style="display:block;width:100%;max-width:220px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="margin-top:22px"><label class="field-label" for="setting-log-channel">LOGGING CHANNEL ID (OPTIONAL)</label>
            <input id="setting-log-channel" type="text" placeholder="e.g. 123456789012345678" style="display:block;width:100%;max-width:420px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="display:flex;align-items:center;gap:16px;margin-top:26px"><button class="button button-primary" type="submit">Save server settings →</button><span id="live-settings-status" aria-live="polite"></span></div>
        </form>
      </div>
      <p style="color:#858990;font-size:12px;margin-top:14px">These settings are stored by the dashboard backend. Applying them to live moderation behavior is part of the later bot integration.</p>`;
    document.getElementById("live-settings-form").addEventListener("submit", saveSettings);
  }

  function selectedValues(optionsId) {
    return [...document.querySelectorAll("#" + optionsId + " input[type=checkbox]:checked")].map(input => input.value);
  }

  function setMultiSelect(optionsId, values) {
    const selected = new Set(values || []);
    document.querySelectorAll("#" + optionsId + " input[type=checkbox]").forEach(input => {
      input.checked = selected.has(input.value);
    });
    updateProtectionDropdown(optionsId);
  }

  function updateProtectionDropdown(optionsId) {
    const isRoles = optionsId === "protection-roles-options";
    const label = isRoles ? "roles" : "channels";
    const summary = document.getElementById(isRoles ? "protection-roles-summary" : "protection-channels-summary");
    if (!summary) return;
    const checked = selectedValues(optionsId);
    summary.textContent = checked.length ? checked.length + " " + label + " selected" : "Choose " + label;
  }

  function renderProtectionDropdown(optionsId, items, prefix = "") {
    const container = document.getElementById(optionsId);
    if (!items.length) {
      container.replaceChildren(Object.assign(document.createElement("p"), { className: "dropdown-empty", textContent: "Nothing available." }));
      return;
    }
    container.replaceChildren(...items.map(item => {
      const label = document.createElement("label");
      label.className = "protection-dropdown-option";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.value = item.id;
      checkbox.addEventListener("change", () => updateProtectionDropdown(optionsId));
      const name = document.createElement("span");
      name.textContent = prefix + item.name;
      label.append(checkbox, name);
      return label;
    }));
  }

  async function loadProtectionOptions() {
    if (!activeGuild) return;
    const status = document.getElementById("protection-status");
    try {
      const data = await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/protection-options");
      renderProtectionDropdown("protection-roles-options", data.roles);
      renderProtectionDropdown("protection-channels-options", data.channels, "#");
      if (currentSettings) {
        setMultiSelect("protection-roles-options", currentSettings.exempt_role_ids);
        setMultiSelect("protection-channels-options", currentSettings.ignored_channel_ids);
      }
    } catch (error) {
      if (status) status.textContent = "Could not load Discord roles/channels: " + error.message;
    }
  }

  function populateProtectionForm(settings) {
    const name = document.getElementById("protection-server-name");
    if (name) name.textContent = activeGuild?.name || "your server";
    const values = {
      "protection-enabled": settings.enabled,
      "protection-text": settings.text_detection,
      "protection-image": settings.image_detection,
      "protection-delete": settings.auto_delete,
      "protection-timeout": settings.auto_timeout,
      "protection-admins": settings.exempt_admins,
      "protection-dry-run": settings.dry_run
    };
    Object.entries(values).forEach(([id, value]) => {
      const input = document.getElementById(id);
      if (input) input.checked = Boolean(value);
    });
    const threshold = document.getElementById("protection-threshold");
    if (threshold) {
      threshold.value = settings.delete_threshold;
      document.getElementById("protection-threshold-value").textContent = (Number(settings.delete_threshold) * 100).toFixed(1) + "%";
    }
    const timeout = document.getElementById("protection-timeout-days");
    if (timeout) timeout.value = Math.min(7, Math.max(1, Number(settings.timeout_days) || 7));
    setMultiSelect("protection-roles-options", settings.exempt_role_ids);
    setMultiSelect("protection-channels-options", settings.ignored_channel_ids);
  }

  function protectionPayload() {
    const base = currentSettings || {};
    return {
      ...base,
      enabled: document.getElementById("protection-enabled").checked,
      text_detection: document.getElementById("protection-text").checked,
      image_detection: document.getElementById("protection-image").checked,
      delete_threshold: Number(document.getElementById("protection-threshold").value),
      auto_delete: document.getElementById("protection-delete").checked,
      auto_timeout: document.getElementById("protection-timeout").checked,
      timeout_days: Math.min(7, Math.max(1, Number(document.getElementById("protection-timeout-days").value))),
      dry_run: document.getElementById("protection-dry-run").checked,
      exempt_admins: document.getElementById("protection-admins").checked,
      exempt_role_ids: selectedValues("protection-roles-options"),
      ignored_channel_ids: selectedValues("protection-channels-options"),
      logging_channel_id: base.logging_channel_id || null
    };
  }

  async function saveProtectionSettings(event) {
    event.preventDefault();
    if (!activeGuild) return;
    const status = document.getElementById("protection-status");
    status.textContent = "Saving…";
    const payload = protectionPayload();
    try {
      const saved = await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/settings", {
        method: "PUT", body: JSON.stringify(payload)
      });
      currentSettings = saved.settings;
      status.textContent = "Saved successfully";
      renderOverview();
      showToast("Protection settings saved.");
    } catch (error) {
      status.textContent = "Could not save: " + error.message;
    }
  }

  async function loadSettings() {
    if (!activeGuild) return;
    const status = document.getElementById("live-settings-status");
    if (status) status.textContent = "Loading…";
    try {
      const data = await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/settings");
      currentSettings = data.settings;
      await loadProtectionOptions();
      const name = document.getElementById("settings-server-name");
      if (name) name.textContent = activeGuild.name;
      const enabled = document.getElementById("setting-enabled");
      if (enabled) enabled.checked = data.settings.enabled;
      const timeout = document.getElementById("setting-timeout");
      if (timeout) timeout.value = Math.min(7, Math.max(1, Number(data.settings.timeout_days) || 7));
      const threshold = document.getElementById("setting-threshold");
      if (threshold) threshold.value = data.settings.delete_threshold;
      const channel = document.getElementById("setting-log-channel");
      if (channel) channel.value = data.settings.logging_channel_id || "";
      populateProtectionForm(data.settings);
      if (status) status.textContent = data.updated_at ? "Saved settings loaded" : "Default settings · not saved yet";
      const protectionStatus = document.getElementById("protection-status");
      if (protectionStatus) protectionStatus.textContent = data.updated_at ? "Saved settings loaded" : "Default settings · not saved yet";
      renderOverview();
    } catch (error) {
      if (status) status.textContent = error.message;
      document.getElementById("overview-protection-status").innerHTML = '<div class="shield-mark">!</div><div><strong>Configuration unavailable</strong><small>' + error.message + '</small></div>';
    }
  }

  async function saveSettings(event) {
    event.preventDefault();
    const status = document.getElementById("live-settings-status");
    status.textContent = "Saving…";
    const payload = {
      ...(currentSettings || {}),
      enabled: document.getElementById("setting-enabled").checked,
      timeout_days: Math.min(7, Math.max(1, Number(document.getElementById("setting-timeout").value))),
      delete_threshold: Number(document.getElementById("setting-threshold").value),
      logging_channel_id: document.getElementById("setting-log-channel").value.trim() || null
    };
    try {
      const saved = await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/settings", { method: "PUT", body: JSON.stringify(payload) });
      currentSettings = saved.settings;
      status.textContent = "Saved successfully";
      renderOverview();
      showToast("Server settings saved.");
    } catch (error) { status.textContent = error.message; }
  }

  function renderRuntimeWarning() {
    const warning = document.getElementById("overview-beta-notice");
    if (warning) warning.hidden = !betaBotPresent;
  }

  async function loadBetaStatus() {
    if (!activeGuild) return;
    const guildId = activeGuild.id;
    const requestId = ++betaStatusRequest;
    betaBotPresent = false;
    renderRuntimeWarning();
    try {
      const status = await api("/api/guilds/" + encodeURIComponent(guildId) + "/beta-status");
      if (requestId !== betaStatusRequest || activeGuild?.id !== guildId) return;
      betaBotPresent = status.beta_bot_present === true;
    } catch (_) {
      if (requestId !== betaStatusRequest) return;
      betaBotPresent = false;
    }
    renderRuntimeWarning();
  }

  function renderOverview() {
    if (!activeGuild) return;
    renderRuntimeWarning();
    document.getElementById("overview-title").textContent = activeGuild.name + " overview";
    document.getElementById("overview-subtitle").textContent = "Security and protection information for " + activeGuild.name + ".";
    const chip = document.getElementById("overview-protection-chip");
    const enabled = currentSettings?.enabled;
    chip.innerHTML = '<i></i> ' + (enabled === undefined ? "Configuration unavailable" : enabled ? "Protection enabled" : "Protection disabled");
    chip.classList.toggle("status-disabled", enabled === false);
    const status = document.getElementById("overview-protection-status");
    const icon = document.createElement("img");
    icon.className = "overview-lock-icon";
    icon.src = asset(enabled === false ? "lock_unsecure.svg" : "lock_secure.svg");
    icon.alt = enabled === false ? "Protection disabled" : "Protection enabled";
    const copy = document.createElement("div");
    const heading = document.createElement("strong");
    heading.textContent = enabled === undefined ? "Configuration unavailable" : enabled ? "Aegis protection enabled" : "Aegis protection disabled";
    const sub = document.createElement("small");
    sub.textContent = "Saved dashboard configuration · bot integration pending";
    copy.append(heading, sub);
    status.replaceChildren(icon, copy);
    const enabledLabel = document.getElementById("overview-enabled");
    enabledLabel.textContent = enabled === undefined ? "UNKNOWN" : enabled ? "ENABLED" : "DISABLED";
    enabledLabel.className = enabled ? "enabled" : "muted-state";
    document.getElementById("overview-timeout-days").textContent = currentSettings ? currentSettings.timeout_days + " DAYS" : "—";
    document.getElementById("overview-threshold").textContent = currentSettings ? (Number(currentSettings.delete_threshold) * 100).toFixed(1) + "%" : "—";
  }

  function enterServer(guild) {
    activeGuild = guild;
    previewServerName = guild.name;
    updatePreview();
    currentSettings = null;
    const avatar = document.getElementById("sidebar-server-avatar");
    avatar.replaceChildren();
    if (guild.icon) {
      const img = document.createElement("img");
      img.src = "https://cdn.discordapp.com/icons/" + encodeURIComponent(guild.id) + "/" + encodeURIComponent(guild.icon) + ".png?size=64";
      img.alt = "";
      img.addEventListener("error", () => { avatar.textContent = initials(guild.name); }, { once: true });
      avatar.appendChild(img);
    } else avatar.textContent = initials(guild.name);
    document.getElementById("sidebar-server-name").textContent = guild.name;
    document.querySelector(".server-meta small").textContent = "Selected server";
    const context = document.querySelector(".server-context");
    if (context) context.textContent = guild.name;
    document.querySelector(".connection").innerHTML = "<i></i> DISCORD CONNECTED";
    settingsForm();
    navigate("overview");
    loadSettings();
    loadBetaStatus();
    if (document.getElementById("page-moderation").classList.contains("active")) loadModeration();
  }

  async function initialize() {
    try {
      const data = await api("/api/me");
      signedIn = true;
      const loading = document.getElementById("selector-loading");
      if (loading) loading.hidden = true;
      renderSelector(data.guilds || []);
      const profile = document.querySelector(".profile-meta");
      if (profile) { profile.querySelector("strong").textContent = data.user.username; profile.querySelector("small").textContent = "Discord account"; }
      const avatar = document.getElementById("profile-avatar");
      if (avatar) {
        avatar.textContent = initials(data.user.username);
        if (data.user.avatar) {
          const ext = data.user.avatar.startsWith("a_") ? "gif" : "png";
          const image = document.createElement("img");
          image.src = "https://cdn.discordapp.com/avatars/" + encodeURIComponent(data.user.id) + "/" + encodeURIComponent(data.user.avatar) + "." + ext + "?size=64";
          image.alt = "";
          image.addEventListener("error", () => { avatar.textContent = initials(data.user.username); }, { once: true });
          avatar.replaceChildren(image);
        }
      }
      document.querySelector(".connection").innerHTML = "<i></i> DISCORD CONNECTED";
      navigate("selector");
    } catch (error) {
      document.getElementById("selector-loading").hidden = true;
      document.getElementById("selector-signin").hidden = false;
      document.querySelector(".connection").innerHTML = "<i></i> SIGN-IN REQUIRED";
      const profile = document.querySelector(".profile-meta strong");
      if (profile) profile.textContent = "Not signed in";
      navigate("selector");
    }
  }

  document.getElementById("protection-form")?.addEventListener("submit", saveProtectionSettings);
  document.getElementById("protection-threshold")?.addEventListener("input", event => {
    document.getElementById("protection-threshold-value").textContent = (Number(event.target.value) * 100).toFixed(1) + "%";
  });

  settingsForm();
  await initialize();
})();
