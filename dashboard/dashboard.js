const pages = [...document.querySelectorAll(".page")];
const navLinks = [...document.querySelectorAll("[data-page]")];
const breadcrumb = document.getElementById("breadcrumb-current");
const toast = document.getElementById("toast");
let toastTimer;
function showToast(message){toast.textContent=message;toast.classList.add("show");clearTimeout(toastTimer);toastTimer=setTimeout(()=>toast.classList.remove("show"),3400)}
function navigate(page){const target=document.getElementById("page-"+page);if(!target)return;pages.forEach(p=>p.classList.toggle("active",p===target));navLinks.forEach(link=>link.classList.toggle("active",link.dataset.page===page));breadcrumb.textContent=page==="owner"?"Owner console":page==="account"?"My account":page.charAt(0).toUpperCase()+page.slice(1);history.replaceState(null,"","#"+page);document.getElementById("sidebar").classList.remove("open");window.scrollTo({top:0,behavior:"smooth"})}
navLinks.forEach(link=>link.addEventListener("click",event=>{event.preventDefault();navigate(link.dataset.page)}));
document.querySelectorAll("[data-page-link]").forEach(button=>button.addEventListener("click",()=>navigate(button.dataset.pageLink)));
document.getElementById("mobile-menu").addEventListener("click",()=>document.getElementById("sidebar").classList.toggle("open"));
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
function updatePreview(){const example={user:"@sample_member",user_id:"123456789012345678",server:"Example Server",reason:"Image attachment matched a known scam campaign.",duration:"7",channel:"<#123456789012345678>",flagged_content:"[Flagged message content]",report_server:"Not configured",deleted:"true",timeout_status:"Timed out successfully"};let value=editor.value;for(const [key,val] of Object.entries(example))value=value.replaceAll("{"+key+"}",val);document.getElementById("message-preview").textContent=value}
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
const initial=location.hash.replace("#","");if(initial&&document.getElementById("page-"+initial))navigate(initial);else navigate("overview");

// Live Discord login and per-server settings.
(async function dashboardBackend() {
  const picker = document.createElement("select");
  picker.id = "guild-picker";
  picker.setAttribute("aria-label", "Choose a Discord server");
  picker.style.cssText = "display:none;margin:10px 16px;width:calc(100% - 32px);padding:10px;background:#111419;color:#f3f3f3;border:1px solid #25282d;border-radius:6px";
  document.getElementById("server-picker").insertAdjacentElement("afterend", picker);

  const ownerCard = document.querySelector(".owner-card");
  if (ownerCard) ownerCard.remove();
  const ownerNav = document.querySelector('[data-page="owner"]');
  if (ownerNav) ownerNav.remove();

  let activeGuild = null;
  let signedIn = false;
  const settingsPage = document.getElementById("page-settings");

  async function api(path, options = {}) {
    const response = await fetch(path, { credentials: "same-origin", ...options,
      headers: { "Content-Type": "application/json", ...(options.headers || {}) } });
    if (response.status === 401) {
      signedIn = false;
      throw new Error("Please sign in with Discord again.");
    }
    if (!response.ok) throw new Error("Request failed (" + response.status + ")");
    return response.json();
  }

  function settingsForm() {
    settingsPage.innerHTML = `
      <div class="page-heading"><div><div class="eyebrow">SERVER CONFIGURATION / SETTINGS</div><h1>Server settings</h1><p>Settings are saved separately for each Discord server.</p></div></div>
      <div class="panel" style="padding:24px;max-width:760px">
        <div id="settings-server-name" style="font-weight:700;margin-bottom:22px"></div>
        <form id="live-settings-form">
          <label class="field-label" style="display:block;margin-bottom:8px"><input id="setting-enabled" type="checkbox" checked> Enable Aegis for this server</label>
          <div style="margin-top:22px"><label class="field-label" for="setting-timeout">DEFAULT TIMEOUT DURATION (DAYS)</label>
            <input id="setting-timeout" type="number" min="1" max="30" step="1" value="7" style="display:block;width:100%;max-width:220px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="margin-top:22px"><label class="field-label" for="setting-threshold">IMAGE MATCH CONFIDENCE THRESHOLD</label>
            <input id="setting-threshold" type="number" min="0.5" max="1" step="0.005" value="0.805" style="display:block;width:100%;max-width:220px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="margin-top:22px"><label class="field-label" for="setting-log-channel">LOGGING CHANNEL ID (OPTIONAL)</label>
            <input id="setting-log-channel" type="text" placeholder="e.g. 123456789012345678" style="display:block;width:100%;max-width:420px;padding:12px;background:#0d0f12;color:#f3f3f3;border:1px solid #25282d;border-radius:5px"></div>
          <div style="display:flex;align-items:center;gap:16px;margin-top:26px"><button class="button button-primary" type="submit">Save server settings →</button><span id="live-settings-status" aria-live="polite"></span></div>
        </form>
      </div>
      <p style="color:#858990;font-size:12px;margin-top:14px">These settings are currently stored by the dashboard backend. Applying them to live moderation behavior is the next integration step.</p>`;
    document.getElementById("live-settings-form").addEventListener("submit", saveSettings);
  }

  async function loadSettings() {
    if (!activeGuild) return;
    const status = document.getElementById("live-settings-status");
    status.textContent = "Loading…";
    try {
      const data = await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/settings");
      document.getElementById("settings-server-name").textContent = activeGuild.name;
      document.getElementById("setting-enabled").checked = data.settings.enabled;
      document.getElementById("setting-timeout").value = data.settings.timeout_days;
      document.getElementById("setting-threshold").value = data.settings.delete_threshold;
      document.getElementById("setting-log-channel").value = data.settings.logging_channel_id || "";
      status.textContent = data.updated_at ? "Saved settings loaded" : "Default settings · not saved yet";
    } catch (error) { status.textContent = error.message; }
  }

  async function saveSettings(event) {
    event.preventDefault();
    const status = document.getElementById("live-settings-status");
    status.textContent = "Saving…";
    const payload = {
      enabled: document.getElementById("setting-enabled").checked,
      timeout_days: Number(document.getElementById("setting-timeout").value),
      delete_threshold: Number(document.getElementById("setting-threshold").value),
      logging_channel_id: document.getElementById("setting-log-channel").value.trim() || null
    };
    try {
      await api("/api/guilds/" + encodeURIComponent(activeGuild.id) + "/settings", {
        method: "PUT", body: JSON.stringify(payload)
      });
      status.textContent = "Saved successfully";
      showToast("Server settings saved.");
    } catch (error) { status.textContent = error.message; }
  }

  async function initialize() {
    try {
      const data = await api("/api/me");
      signedIn = true;
      const manageable = data.guilds.filter(g => g.owner || ((BigInt(g.permissions || "0") & 40n) !== 0n));
      picker.style.display = manageable.length ? "block" : "none";
      picker.innerHTML = "";
      manageable.forEach(guild => {
        const option = document.createElement("option");
        option.value = guild.id; option.textContent = guild.name; picker.appendChild(option);
      });
      if (!manageable.length) {
        settingsPage.innerHTML = '<div class="panel" style="padding:24px"><h2>No manageable servers found</h2><p>Your Discord account needs Manage Server or Administrator permission in a server where Aegis can be configured.</p></div>';
        return;
      }
      activeGuild = manageable[0];
      picker.value = activeGuild.id;
      picker.addEventListener("change", () => {
        activeGuild = manageable.find(g => g.id === picker.value);
        loadSettings();
      });
      document.querySelector(".server-meta strong").textContent = activeGuild.name;
      document.querySelector(".server-meta small").textContent = "Connected Discord server";
      document.querySelector(".connection").innerHTML = "<i></i> DISCORD CONNECTED";
      settingsForm();
      await loadSettings();
      const notice = document.querySelector(".notice");
      if (notice && notice.textContent.includes("Dashboard preview")) notice.remove();
      const profile = document.querySelector(".profile-meta");
      if (profile) { profile.querySelector("strong").textContent = data.user.username; profile.querySelector("small").textContent = "Discord account"; }
      const avatar = document.querySelector(".profile-avatar");
      if (avatar) {
        avatar.textContent = data.user.username.slice(0, 1).toUpperCase();
        if (data.user.avatar) {
          const extension = data.user.avatar.startsWith("a_") ? "gif" : "png";
          const image = document.createElement("img");
          image.src = `https://cdn.discordapp.com/avatars/${encodeURIComponent(data.user.id)}/${encodeURIComponent(data.user.avatar)}.${extension}?size=64`;
          image.alt = "";
          image.draggable = false;
          image.addEventListener("error", () => {
            image.remove();
            avatar.textContent = data.user.username.slice(0, 1).toUpperCase();
          }, { once: true });
          avatar.replaceChildren(image);
        }
      }
    } catch (error) {
      document.querySelector(".connection").innerHTML = "<i></i> SIGN-IN REQUIRED";
      settingsPage.innerHTML = '<div class="panel" style="padding:24px"><h2>Discord account not connected</h2><p>Open the Aegis website and use its dashboard link to connect your Discord account. You will be returned here after authorization.</p></div>';
      const demoName = document.querySelector(".profile-meta strong");
      if (demoName) demoName.textContent = "Not signed in";
      if (ownerNav) ownerNav.remove();
    }
  }

  settingsForm();
  await initialize();
})();
