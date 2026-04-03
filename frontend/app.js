'use strict';

// ─── Tauri IPC ────────────────────────────────────────────────────────────────
const { invoke } = window.__TAURI__.core;
const appWindow  = window.__TAURI__.window.getCurrentWindow();

// ─── State ────────────────────────────────────────────────────────────────────
let allAccounts = [];

// ─── DOM Refs ─────────────────────────────────────────────────────────────────
const cardsGrid       = document.getElementById('cards-grid');
const emptyState      = document.getElementById('empty-state');
const addBtn          = document.getElementById('add-account-btn');
const refreshAllBtn   = document.getElementById('refresh-all-btn');
const lobbyViewerBtn  = document.getElementById('lobby-viewer-btn');
const sortSelect      = document.getElementById('sort-select');
const accountModal    = document.getElementById('account-modal');
const settingsPanel   = document.getElementById('settings-panel');
const modalTitle      = document.getElementById('modal-title');
const editIdInput     = document.getElementById('edit-id');
const toastContainer  = document.getElementById('toast-container');
const lobbyModal      = document.getElementById('lobby-modal');
const lobbyModalTitle = document.getElementById('lobby-modal-title');
const lobbyMeta       = document.getElementById('lobby-meta');
const lobbyList       = document.getElementById('lobby-list');
const lobbyError      = document.getElementById('lobby-error');

let activeLobbyData = null;
let currentSortMode = localStorage.getItem('accountsSortMode') || 'highest';

// ─── Tier helpers ─────────────────────────────────────────────────────────────
const TIER_COLORS = {
  IRON: '#6b6b6b', BRONZE: '#8c5a2c', SILVER: '#9aa4af',
  GOLD: '#cd8400', PLATINUM: '#4fa89e', EMERALD: '#1ba94c',
  DIAMOND: '#576bce', MASTER: '#9d48e0', GRANDMASTER: '#d4373e',
  CHALLENGER: '#f4c874', UNRANKED: '#4a5a6a'
};

function tierClass(tier) {
  return `tier-${(tier || 'UNRANKED').toUpperCase()}`;
}

const TIER_ORDER = {
  CHALLENGER: 9, GRANDMASTER: 8, MASTER: 7, DIAMOND: 6, EMERALD: 5,
  PLATINUM: 4, GOLD: 3, SILVER: 2, BRONZE: 1, IRON: 0, UNRANKED: -1
};

const DIVISION_ORDER = { I: 4, II: 3, III: 2, IV: 1 };

function rankScoreFromStats(stats) {
  const solo = stats?.solo || { tier: 'UNRANKED' };
  const flex = stats?.flex || { tier: 'UNRANKED' };
  const candidates = [solo, flex];
  const scoreFor = (rank) => {
    const tier = (rank?.tier || 'UNRANKED').toUpperCase();
    const tierScore = TIER_ORDER[tier] ?? -1;
    if (tierScore < 0) return -1;
    const div = DIVISION_ORDER[(rank?.division || '').toUpperCase()] ?? 0;
    const lp = Number(rank?.lp || 0);
    return tierScore * 1000 + div * 100 + lp;
  };
  return Math.max(...candidates.map(scoreFor));
}

function capitalise(str) {
  if (!str) return '';
  return str.charAt(0).toUpperCase() + str.slice(1).toLowerCase();
}

function formatRankText(rank) {
  if (!rank || rank.tier === 'UNRANKED') return 'Unranked';
  const highTier = ['MASTER', 'GRANDMASTER', 'CHALLENGER'].includes(rank.tier);
  const div = highTier ? '' : ' ' + rank.division;
  return `${capitalise(rank.tier)}${div} — ${rank.lp} LP`;
}

function rankIconSrc(tier) {
  const t = (tier || 'unranked').toLowerCase();
  return `assets/rank-icons/${t}.svg`;
}

function winRateHTML(rank) {
  if (!rank || rank.tier === 'UNRANKED' || rank.winRate === null) {
    return '<span class="wr-unranked">No ranked data</span>';
  }
  const pct = parseFloat(rank.winRate);
  const color = pct >= 50 ? 'var(--success)' : 'var(--danger)';
  return `
    <span class="wr-wins">${rank.wins}W</span>
    <span class="wr-losses">${rank.losses}L</span>
    <span class="wr-pct" style="color:${color}">${rank.winRate}%</span>
  `;
}

// ─── HTML Escaping ────────────────────────────────────────────────────────────
function esc(str) {
  return String(str ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// ─── Profile URL Builders ─────────────────────────────────────────────────────
const OPGG_REGIONS = {
  NA: 'na', EUW: 'euw', EUNE: 'eune', KR: 'kr', JP: 'jp',
  BR: 'br', LAN: 'lan', LAS: 'las', OCE: 'oce', TR: 'tr', RU: 'ru'
};
const UGG_SERVERS = {
  NA: 'na1', EUW: 'euw1', EUNE: 'eun1', KR: 'kr', JP: 'jp1',
  BR: 'br1', LAN: 'la1', LAS: 'la2', OCE: 'oc1', TR: 'tr1', RU: 'ru'
};

function buildOpggUrl(riotId, region) {
  const name = riotId.replace('#', '-');
  const r = OPGG_REGIONS[region] || region.toLowerCase();
  return `https://www.op.gg/summoners/${r}/${encodeURIComponent(name)}`;
}

function buildUggUrl(riotId, region) {
  const name = riotId.replace('#', '-');
  const s = UGG_SERVERS[region] || `${region.toLowerCase()}1`;
  return `https://u.gg/lol/profile/${s}/${encodeURIComponent(name)}/overview`;
}

function buildOpggMultiUrl(players, region) {
  const r = (region || 'na').toLowerCase();
  const names = players
    .map(p => (p.gameName && p.tagLine) ? `${p.gameName}#${p.tagLine}` : null)
    .filter(Boolean);
  return `https://www.op.gg/multisearch/${r}?summoners=${encodeURIComponent(names.join(','))}`;
}

function buildTrackerMultiUrl(players, region) {
  const r = (region || 'na').toLowerCase();
  const names = players
    .map(p => (p.gameName && p.tagLine) ? `${p.gameName}#${p.tagLine}` : null)
    .filter(Boolean);
  return `https://tracker.gg/lol/multisearch/${r}/${encodeURIComponent(names.join(','))}`;
}

// ─── Data Dragon champion icons ───────────────────────────────────────────────
let DD_VERSION = '16.5.1';
const DD_VERSION_CACHE_KEY = 'ddragonVersionCacheV1';
const DD_VERSION_CACHE_MAX_AGE_MS = 24 * 60 * 60 * 1000; // 24h

async function initDataDragonVersion() {
  try {
    const cachedRaw = localStorage.getItem(DD_VERSION_CACHE_KEY);
    if (cachedRaw) {
      const cached = JSON.parse(cachedRaw);
      if (cached?.version && typeof cached.version === 'string' && (Date.now() - (cached.ts || 0)) < DD_VERSION_CACHE_MAX_AGE_MS) {
        DD_VERSION = cached.version;
        return;
      }
    }

    const res = await fetch('https://ddragon.leagueoflegends.com/api/versions.json', { cache: 'no-store' });
    if (!res.ok) throw new Error(`versions.json HTTP ${res.status}`);
    const versions = await res.json();
    const latest = Array.isArray(versions) ? versions[0] : null;
    if (latest && typeof latest === 'string') {
      DD_VERSION = latest;
      localStorage.setItem(DD_VERSION_CACHE_KEY, JSON.stringify({ version: latest, ts: Date.now() }));
    }
  } catch {
    // best-effort: keep fallback version
  }
}

// Champions whose Data Dragon key differs from the display name
const DD_OVERRIDES = {
  "Wukong":          "MonkeyKing",
  "Renata Glasc":    "Renata",
  "Nunu & Willump":  "Nunu",
  "LeBlanc":         "Leblanc",
  "Dr. Mundo":       "DrMundo",
  "Jarvan IV":       "JarvanIV",
  "Kai'Sa":          "Kaisa",
  "Kha'Zix":         "Khazix",
  "Cho'Gath":        "Chogath",
  "Kog'Maw":         "KogMaw",
  "Vel'Koz":         "Velkoz",
  "Bel'Veth":        "Belveth",
  "Rek'Sai":         "RekSai",
  "K'Sante":         "KSante",
  "Tahm Kench":      "TahmKench",
  "Aurelion Sol":    "AurelionSol",
  "Lee Sin":         "LeeSin",
  "Master Yi":       "MasterYi",
  "Miss Fortune":    "MissFortune",
  "Twisted Fate":    "TwistedFate",
  "Xin Zhao":        "XinZhao",
};

function champDDKey(name) {
  if (DD_OVERRIDES[name]) return DD_OVERRIDES[name];
  // Default: capitalise each word, strip non-alphanumeric
  return name.split(/\s+/)
    .map(w => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')
    .replace(/[^a-zA-Z0-9]/g, '');
}

function champIconUrl(name) {
  return `https://ddragon.leagueoflegends.com/cdn/${DD_VERSION}/img/champion/${champDDKey(name)}.png`;
}

function buildChampionsHTML(champs) {
  if (!champs || champs.length === 0) return '';
  const rows = champs.slice(0, 3).map(c => {
    const wr      = c.winRate != null ? c.winRate.toFixed(0) : null;
    const wrColor = c.winRate != null && c.winRate >= 50 ? 'var(--success)' : 'var(--danger)';
    const wins    = c.winRate != null ? Math.round(c.games * c.winRate / 100) : null;
    const losses  = wins != null ? c.games - wins : null;
    const recordHTML = wins != null
      ? `<span class="champ-record">${wins}W / ${losses}L</span>`
      : '';
    const wrHTML = wr != null
      ? `<span class="champ-wr" style="color:${wrColor}">${wr}%</span>`
      : `<span class="champ-wr" style="color:var(--text-dim)">—</span>`;
    return `
      <div class="champ-row">
        <img class="champ-icon" src="${esc(champIconUrl(c.name))}"
             alt="${esc(c.name)}"
             onerror="this.style.opacity='0.3'">
        <span class="champ-name">${esc(c.name)}</span>
        <span class="champ-games">${c.games}G</span>
        ${recordHTML}
        ${wrHTML}
      </div>`;
  }).join('');
  return `
    <div class="card-champs-v2">
      <div class="champs-header">TOP 3 CHAMPIONS</div>
      ${rows}
    </div>`;
}

// ─── Queue Block Builder ──────────────────────────────────────────────────────
function buildQueueBlock(label, rank) {
  if (!rank || rank.tier === 'UNRANKED') {
    return `
      <div class="queue-block">
        <div class="queue-label">${label}</div>
        <div class="queue-rank tier-UNRANKED">Unranked</div>
        <div class="queue-nodata">No data</div>
      </div>`;
  }
  const tier = (rank.tier || 'UNRANKED').toUpperCase();
  const highTier = ['MASTER', 'GRANDMASTER', 'CHALLENGER'].includes(tier);
  const divMap = { 'I': '1', 'II': '2', 'III': '3', 'IV': '4' };
  const divNum = divMap[rank.division] || rank.division || '';
  const tierStr = highTier ? capitalise(rank.tier) : `${capitalise(rank.tier)} ${divNum}`;
  const pct = parseFloat(rank.winRate) || 0;
  const wrColor = pct >= 50 ? 'var(--success)' : 'var(--danger)';
  return `
    <div class="queue-block">
      <div class="queue-label">${label}</div>
      <div class="queue-rank ${tierClass(rank.tier)}">${tierStr}</div>
      <div class="queue-lp">${rank.lp} LP</div>
      <div class="queue-record">${rank.wins}W / ${rank.losses}L</div>
      <div class="queue-wr" style="color:${wrColor}">${rank.winRate}%</div>
    </div>`;
}

// ─── Card Rendering ───────────────────────────────────────────────────────────
function buildCard(account) {
  const stats    = account.stats;
  const iconUrl  = stats?.iconUrl  || 'assets/default-icon.svg';
  const level    = stats?.summonerLevel ?? '—';
  const solo     = stats?.solo  || { tier: 'UNRANKED' };
  const flex     = stats?.flex  || { tier: 'UNRANKED' };
  const champs   = stats?.topChampions || [];
  const topTier  = solo.tier !== 'UNRANKED' ? solo.tier : flex.tier;

  const card = document.createElement('div');
  card.className = `account-card ${tierClass(topTier)}`;
  card.dataset.id = account.id;

  card.innerHTML = `
    <div class="card-hero">
      <div class="card-icon-wrap rank-${esc(topTier)}">
        <img class="card-icon" src="${esc(iconUrl)}" alt="icon"
             onerror="this.src='assets/default-icon.svg'">
      </div>
      <div class="card-label">${esc(account.label)}</div>
      <div class="card-riotid">${esc(account.riotId)}</div>
      <span class="card-region-tag">${esc(account.region)}</span>
    </div>

    <div class="card-credentials">
      <div class="cred-row">
        <span class="cred-label">Username</span>
        <span class="cred-value">${esc(account.username)}</span>
        <div class="cred-actions">
          <button class="copy-icon-btn js-copy-user" data-id="${esc(account.id)}" title="Copy username"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>
        </div>
      </div>
      <div class="cred-row">
        <span class="cred-label">Password</span>
        <span class="cred-value password js-pw-display" data-id="${esc(account.id)}" data-revealed="false">••••••••</span>
        <div class="cred-actions">
          <button class="copy-icon-btn js-copy-pw" data-id="${esc(account.id)}" title="Copy password"><svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg></button>
          <button class="copy-icon-btn js-toggle-pw" data-id="${esc(account.id)}" title="Show/Hide"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg></button>
        </div>
      </div>
    </div>

    <div class="card-actions">
      <button class="action-btn play js-launch" data-id="${esc(account.id)}" title="Open Riot Client"><svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><polygon points="5 3 19 12 5 21 5 3"/></svg></button>
      <button class="action-btn refresh js-refresh" data-id="${esc(account.id)}" title="Refresh stats"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg></button>
      <button class="action-btn edit js-edit" data-id="${esc(account.id)}" title="Edit account"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></svg></button>
      <button class="action-btn delete js-delete" data-id="${esc(account.id)}" title="Delete account"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"/></svg></button>
    </div>

    <div class="card-stats">
      ${buildQueueBlock('SOLO / DUO', solo)}
      ${buildQueueBlock('FLEX QUEUE', flex)}
    </div>

    ${buildChampionsHTML(champs)}

    <div class="card-footer">
      <span class="card-footer-ts">${stats?.fetchedAt
        ? `LAST UPDATED: ${new Date(stats.fetchedAt).toLocaleString()}`
        : '<span style="color:var(--text-dim)">No stats — click refresh to fetch</span>'
      }</span>
      <div class="card-profile-links">
        <button class="profile-link js-opgg" data-url="${esc(buildOpggUrl(account.riotId, account.region))}" title="View on op.gg"><img src="assets/opgg-logo.svg" alt="op.gg" draggable="false"></button>
        <button class="profile-link js-ugg"  data-url="${esc(buildUggUrl(account.riotId, account.region))}"  title="View on u.gg"><img src="assets/ugg-logo.svg"  alt="u.gg"  draggable="false"></button>
      </div>
    </div>

    <div class="card-loading hidden" id="card-loading-${esc(account.id)}">
      <div class="spinner"></div> Fetching stats…
    </div>
  `;

  return card;
}

// ─── Render All Cards ─────────────────────────────────────────────────────────
async function renderAllCards() {
  try {
    allAccounts = await invoke('get_all');
  } catch (err) {
    return showToast(String(err), 'error');
  }

  // Clear all dynamic rows (cards + region separators) but keep empty state node.
  cardsGrid.querySelectorAll(':scope > :not(#empty-state)').forEach(n => n.remove());

  if (allAccounts.length === 0) {
    emptyState.classList.remove('hidden');
    return;
  }

  emptyState.classList.add('hidden');
  const sortedAccounts = [...allAccounts];
  if (currentSortMode === 'highest') {
    sortedAccounts.sort((a, b) => rankScoreFromStats(b.stats) - rankScoreFromStats(a.stats));
  } else if (currentSortMode === 'lowest') {
    sortedAccounts.sort((a, b) => rankScoreFromStats(a.stats) - rankScoreFromStats(b.stats));
  } else if (currentSortMode === 'region') {
    sortedAccounts.sort((a, b) => (a.region || '').localeCompare(b.region || '') || a.label.localeCompare(b.label));
  }

  const frag = document.createDocumentFragment();
  let prevRegion = null;
  sortedAccounts.forEach((acc, i) => {
    if (currentSortMode === 'region' && acc.region !== prevRegion) {
      prevRegion = acc.region;
      const separator = document.createElement('div');
      separator.className = 'region-separator';
      separator.innerHTML = `<span>${esc(acc.region || 'Unknown Region')}</span>`;
      frag.appendChild(separator);
    }

    const card = buildCard(acc);
    card.classList.add('is-entering');
    card.style.animationDelay = `${Math.min(i * 35, 220)}ms`;
    frag.appendChild(card);
    setTimeout(() => {
      card.classList.remove('is-entering');
      card.style.animationDelay = '';
    }, 900);
  });
  cardsGrid.appendChild(frag);
}

// ─── Event Delegation (cards grid) ───────────────────────────────────────────
cardsGrid.addEventListener('click', async (e) => {
  const target = e.target;

  const profileBtn = target.closest('.js-opgg, .js-ugg');
  if (profileBtn) {
    const url = profileBtn.dataset.url;
    if (url) invoke('open_external', { url }).catch(() => {});
    return;
  }

  const btn = target.closest('[data-id]');
  if (!btn) return;
  const id = btn.dataset.id;

  if (btn.classList.contains('js-launch')) {
    await handleLaunchAccount(id);
  } else if (btn.classList.contains('js-edit')) {
    openEditModal(id);
  } else if (btn.classList.contains('js-delete')) {
    await handleDelete(id);
  } else if (btn.classList.contains('js-refresh')) {
    await handleRefreshCard(id);
  } else if (btn.classList.contains('js-copy-user')) {
    const acc = allAccounts.find(a => a.id === id);
    if (acc) {
      try {
        await invoke('write_text', { text: acc.username });
        showCopyFeedback(btn, '✓');
      } catch (err) {
        showToast(String(err), 'error');
      }
    }
  } else if (btn.classList.contains('js-copy-pw')) {
    try {
      await invoke('copy_password', { id });
      showCopyFeedback(btn, '✓');
    } catch (err) {
      showToast(String(err) || 'Failed to copy', 'error');
    }
  } else if (btn.classList.contains('js-toggle-pw')) {
    await handleTogglePassword(id);
  }
});

function showCopyFeedback(btn, msg) {
  const original = btn.innerHTML;
  btn.innerHTML = msg;
  btn.classList.add('copied');
  setTimeout(() => { btn.innerHTML = original; btn.classList.remove('copied'); }, 1800);
}

// ─── Password Reveal ──────────────────────────────────────────────────────────
const revealTimers = {};

async function handleTogglePassword(id) {
  const span = cardsGrid.querySelector(`.js-pw-display[data-id="${id}"]`);
  if (!span) return;

  if (span.dataset.revealed === 'true') {
    span.textContent = '••••••••';
    span.dataset.revealed = 'false';
    clearTimeout(revealTimers[id]);
  } else {
    try {
      const password = await invoke('get_password', { id });
      span.textContent = password;
      span.dataset.revealed = 'true';
      clearTimeout(revealTimers[id]);
      revealTimers[id] = setTimeout(() => {
        if (span.dataset.revealed === 'true') {
          span.textContent = '••••••••';
          span.dataset.revealed = 'false';
        }
      }, 15_000);
    } catch (err) {
      showToast(String(err), 'error');
    }
  }
}

// ─── Launch Riot Client ───────────────────────────────────────────────────────
async function handleLaunchAccount(id) {
  const btn = cardsGrid.querySelector(`.js-launch[data-id="${id}"]`);
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;

  if (btn) btn.disabled = true;
  showToast(`Opening Riot Client for "${acc.label}"…`, 'info', 15000);

  try {
    await invoke('launch_account', { id });
    showToast('Riot Client launched.', 'success', 4000);
  } catch (err) {
    showToast(String(err) || 'Launch failed', 'error');
  } finally {
    if (btn) btn.disabled = false;
  }
}

// ─── Delete Confirm Modal ─────────────────────────────────────────────────────
const deleteModal      = document.getElementById('delete-modal');
const deleteModalMsg   = document.getElementById('delete-modal-msg');
const deleteCancelBtn  = document.getElementById('delete-cancel-btn');
const deleteConfirmBtn = document.getElementById('delete-confirm-btn');

let _deleteResolve = null;
deleteCancelBtn.addEventListener('click',  () => { deleteModal.classList.add('hidden'); _deleteResolve?.(false); });
deleteConfirmBtn.addEventListener('click', () => { deleteModal.classList.add('hidden'); _deleteResolve?.(true);  });

function confirmDelete(label) {
  return new Promise(resolve => {
    _deleteResolve = resolve;
    deleteModalMsg.textContent = `Delete "${label}" from the vault? This cannot be undone.`;
    deleteModal.classList.remove('hidden');
  });
}

// ─── Delete Account ───────────────────────────────────────────────────────────
async function handleDelete(id) {
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;
  if (!await confirmDelete(acc.label)) return;
  try {
    await invoke('delete_account', { id });
    showToast(`"${acc.label}" deleted.`, 'info');
    await renderAllCards();
  } catch (err) {
    showToast(String(err) || 'Failed to delete', 'error');
  }
}

// ─── Refresh Single Card ──────────────────────────────────────────────────────
async function handleRefreshCard(id) {
  const card    = cardsGrid.querySelector(`.account-card[data-id="${id}"]`);
  const loading = document.getElementById(`card-loading-${id}`);
  const btn     = card?.querySelector('.js-refresh');

  if (loading) loading.classList.remove('hidden');
  if (btn)     btn.disabled = true;

  try {
    const stats = await invoke('refresh_stats', { id });
    const idx = allAccounts.findIndex(a => a.id === id);
    if (idx !== -1) allAccounts[idx].stats = stats;
    const newCard = buildCard(allAccounts[idx]);
    card?.replaceWith(newCard);
    newCard.classList.add('is-refreshed');
    setTimeout(() => newCard.classList.remove('is-refreshed'), 1100);
    showToast('Stats updated!', 'success');
  } catch (err) {
    if (loading) loading.classList.add('hidden');
    if (btn) btn.disabled = false;
    showToast(String(err) || 'Failed to fetch stats', 'error');
  }
}

// ─── Lobby Viewer ─────────────────────────────────────────────────────────────
document.getElementById('lobby-close-btn')?.addEventListener('click', closeLobbyViewer);
document.getElementById('lobby-done-btn')?.addEventListener('click', closeLobbyViewer);
document.getElementById('lobby-refresh-btn')?.addEventListener('click', async () => {
  await fetchLobby();
});
lobbyViewerBtn?.addEventListener('click', async () => {
  await openLobbyViewer();
});
document.getElementById('lobby-open-opgg-btn')?.addEventListener('click', async () => {
  await openLobbyMultiSite('opgg');
});
document.getElementById('lobby-open-tracker-btn')?.addEventListener('click', async () => {
  await openLobbyMultiSite('tracker');
});
lobbyModal?.addEventListener('click', (e) => {
  if (e.target === lobbyModal) closeLobbyViewer();
});

function closeLobbyViewer() {
  lobbyModal.classList.add('hidden');
  activeLobbyData = null;
}

function renderLobbyPlayers(players) {
  if (!players || players.length === 0) {
    lobbyList.innerHTML = '<div class="hint">No champion-select players found yet.</div>';
    return;
  }

  const friendlyName = (p) => {
    if (p.gameName && p.tagLine) return `${p.gameName}#${p.tagLine}`;
    if (p.summonerName && p.summonerName !== 'Hidden Summoner') return p.summonerName;
    return 'Hidden Summoner';
  };

  const secondary = (p) => {
    if (p.gameName && p.tagLine) return `Riot ID: ${p.gameName}#${p.tagLine}`;
    return 'Riot ID unavailable yet';
  };

  lobbyList.innerHTML = players.map(p => `
    <div class="lobby-row">
      <div>
        <div class="lobby-name">${esc(friendlyName(p))}</div>
        <div class="lobby-puuid">${esc(secondary(p))}</div>
      </div>
    </div>
  `).join('');
}

async function fetchLobby() {
  const refreshBtn = document.getElementById('lobby-refresh-btn');
  if (refreshBtn) refreshBtn.disabled = true;
  lobbyError.classList.add('hidden');
  lobbyMeta.textContent = 'Loading lobby data...';
  lobbyList.innerHTML = '<div class="hint"><span class="spinner-small"></span> Checking League client...</div>';

  try {
    const data = await invoke('get_lobby_view');
    activeLobbyData = data;
    lobbyMeta.textContent = `Phase: ${data.phase} • Region: ${data.region || 'NA'} • Players: ${data.players.length}`;
    if (!data.inChampSelect) {
      lobbyMeta.textContent += ' • Not in champ select';
    }
    renderLobbyPlayers(data.players || []);
  } catch (err) {
    activeLobbyData = null;
    lobbyMeta.textContent = 'Lobby unavailable';
    lobbyList.innerHTML = '';
    lobbyError.textContent = String(err) || 'Failed to load lobby.';
    lobbyError.classList.remove('hidden');
  } finally {
    if (refreshBtn) refreshBtn.disabled = false;
  }
}

async function openLobbyMultiSite(site) {
  if (!activeLobbyData?.players?.length) {
    showToast('No lobby players available for multi-search.', 'info');
    return;
  }
  const eligible = activeLobbyData.players.filter(p => p.gameName && p.tagLine);
  if (eligible.length === 0) {
    showToast('Players are missing Riot IDs (gameName#tagLine).', 'error');
    return;
  }

  const url = site === 'tracker'
    ? buildTrackerMultiUrl(eligible, activeLobbyData.region || 'NA')
    : buildOpggMultiUrl(eligible, activeLobbyData.region || 'NA');

  try {
    await invoke('open_external', { url });
  } catch (err) {
    showToast(String(err) || 'Failed to open browser.', 'error');
  }
}

async function openLobbyViewer() {
  lobbyModalTitle.textContent = 'Lobby Reveal';
  lobbyModal.classList.remove('hidden');
  await fetchLobby();
}

// ─── Auto-accept toggle ───────────────────────────────────────────────────────
const autoAcceptToggle    = document.getElementById('auto-accept-toggle');
const autoAcceptHeaderBtn = document.getElementById('auto-accept-header-btn');

function syncAutoAcceptUI(enabled) {
  if (autoAcceptToggle) autoAcceptToggle.checked = !!enabled;
  if (autoAcceptHeaderBtn) autoAcceptHeaderBtn.classList.toggle('header-btn--active', !!enabled);
}

(async () => {
  try {
    const enabled = await invoke('get_auto_accept_status');
    syncAutoAcceptUI(enabled);
  } catch {
    syncAutoAcceptUI(false);
  }
})();

async function setAutoAccept(enabled) {
  try {
    const result = await invoke('set_auto_accept_enabled', { enabled });
    syncAutoAcceptUI(result);
    showToast(result ? 'Auto-accept enabled.' : 'Auto-accept disabled.', 'info');
  } catch (err) {
    syncAutoAcceptUI(!enabled);
    showToast(String(err) || 'Failed to update auto-accept.', 'error');
  }
}

if (autoAcceptToggle) {
  autoAcceptToggle.addEventListener('change', () => setAutoAccept(autoAcceptToggle.checked));
}
if (autoAcceptHeaderBtn) {
  autoAcceptHeaderBtn.addEventListener('click', () => {
    const next = !autoAcceptHeaderBtn.classList.contains('header-btn--active');
    setAutoAccept(next);
  });
}

if (sortSelect) {
  sortSelect.value = currentSortMode;
  sortSelect.addEventListener('change', async () => {
    currentSortMode = sortSelect.value;
    localStorage.setItem('accountsSortMode', currentSortMode);
    await renderAllCards();
  });
}

// ─── Refresh All ──────────────────────────────────────────────────────────────
refreshAllBtn.addEventListener('click', async () => {
  if (allAccounts.length === 0) return;
  const REFRESH_SVG = `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>`;
  refreshAllBtn.disabled = true;
  refreshAllBtn.innerHTML = REFRESH_SVG;

  try {
    const results = await invoke('refresh_all');
    await renderAllCards();

    const failed = results.filter(r => !r.success).length;
    const ok     = results.filter(r => r.success).length;
    if (failed === 0)  showToast(`All ${ok} account(s) updated!`, 'success');
    else if (ok === 0) showToast('All refreshes failed. Check the Riot IDs and regions.', 'error');
    else               showToast(`${ok} updated, ${failed} failed.`, 'info');
  } catch (err) {
    showToast(String(err) || 'Refresh failed', 'error');
  } finally {
    refreshAllBtn.disabled = false;
    refreshAllBtn.innerHTML = REFRESH_SVG;
  }
});

// ─── Modal: Add / Edit ────────────────────────────────────────────────────────
addBtn.addEventListener('click', openAddModal);
document.getElementById('modal-close-btn').addEventListener('click', closeModal);
document.getElementById('modal-cancel-btn').addEventListener('click', closeModal);
document.getElementById('modal-save-btn').addEventListener('click', handleModalSave);

document.querySelector('.toggle-btn[data-target="field-password"]')?.addEventListener('click', () => {
  const input = document.getElementById('field-password');
  input.type = input.type === 'password' ? 'text' : 'password';
});



document.getElementById('field-region')?.addEventListener('keydown', e => {
  if (e.key === 'Enter') handleModalSave();
});

function openAddModal() {
  modalTitle.textContent = 'Add Account';
  editIdInput.value = '';
  clearModalFields();
  document.getElementById('edit-pw-hint').classList.add('hidden');
  document.getElementById('field-password').placeholder = 'Your League of Legends password';
  accountModal.classList.remove('hidden');
  document.getElementById('field-label').focus();
}

function openEditModal(id) {
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;

  modalTitle.textContent = 'Edit Account';
  editIdInput.value = id;

  document.getElementById('field-label').value    = acc.label;
  document.getElementById('field-username').value  = acc.username;
  document.getElementById('field-password').value  = '';
  document.getElementById('field-riotid').value    = acc.riotId;
  document.getElementById('field-region').value    = acc.region;

  document.getElementById('edit-pw-hint').classList.remove('hidden');
  document.getElementById('field-password').placeholder = '(leave blank to keep existing)';
  document.getElementById('modal-error').classList.add('hidden');

  accountModal.classList.remove('hidden');
  document.getElementById('field-label').focus();
}

function closeModal() {
  accountModal.classList.add('hidden');
  clearModalFields();
}

function clearModalFields() {
  ['field-label', 'field-username', 'field-password', 'field-riotid'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
  document.getElementById('field-region').value = 'NA';
  document.getElementById('field-password').type = 'password';
  document.getElementById('modal-error').classList.add('hidden');
}

function showModalError(msg) {
  const el = document.getElementById('modal-error');
  el.textContent = msg;
  el.classList.remove('hidden');
}

async function handleModalSave() {
  const id       = editIdInput.value;
  const label    = document.getElementById('field-label').value.trim();
  const username = document.getElementById('field-username').value.trim();
  const password = document.getElementById('field-password').value;
  const riotId   = document.getElementById('field-riotid').value.trim();
  const region   = document.getElementById('field-region').value;

  if (!label)    return showModalError('Display label is required.');
  if (!username) return showModalError('Username is required.');
  if (!riotId)   return showModalError('Riot ID is required.');
  if (!/^.+#.+$/.test(riotId)) return showModalError('Riot ID must be in "GameName#TAG" format (e.g. Faker#KR1).');

  const saveBtn = document.getElementById('modal-save-btn');
  saveBtn.disabled = true;

  try {
    if (id) {
      const updates = { label, username, riotId, region };
      if (password) updates.password = password;
      await invoke('update_account', { id, updates });
    } else {
      if (!password) {
        saveBtn.disabled = false;
        return showModalError('Password is required.');
      }
      const newId = await invoke('add_account', { account: { label, username, password, riotId, region } });
      closeModal();
      await renderAllCards();
      showToast('Account added! Fetching stats...', 'success');
      handleRefreshCard(newId);
      return;
    }

    closeModal();
    await renderAllCards();
    showToast('Account updated.', 'success');
  } catch (err) {
    showModalError(String(err) || 'Failed to save.');
  } finally {
    saveBtn.disabled = false;
  }
}

// ─── Theme System ─────────────────────────────────────────────────────────────
let currentTheme = localStorage.getItem('appTheme') || 'default';

function applyTheme(theme) {
  currentTheme = theme;
  localStorage.setItem('appTheme', theme);

  if (theme === 'default') {
    delete document.body.dataset.theme;
  } else {
    document.body.dataset.theme = theme;
  }

  document.querySelectorAll('.theme-dot').forEach(dot => {
    dot.classList.toggle('active', dot.dataset.theme === theme);
  });

  const themeBg = document.getElementById('theme-bg');
  if (!themeBg) return;
  themeBg.innerHTML = '';
  themeBg.className = 'theme-bg';

  if (theme === 'galaxy') {
    createGalaxyBg(themeBg);
  } else if (theme === 'starry') {
    createStarryBg(themeBg);
  }
}

function createGalaxyBg(container) {
  const orbs = [
    { color: '#ff00ff', size: 420, x: 8,  y: 4,  tx:  45, ty: -30, dur:  8, sc: 1.2  },
    { color: '#00ffff', size: 360, x: 68, y: 12,  tx: -50, ty:  42, dur: 11, sc: 1.15 },
    { color: '#ff6600', size: 310, x: 38, y: 58,  tx:  32, ty: -42, dur:  9, sc: 1.1  },
    { color: '#00ff88', size: 330, x: 82, y: 48,  tx: -38, ty:  28, dur: 13, sc: 1.2  },
    { color: '#ff3366', size: 290, x: 22, y: 78,  tx:  48, ty: -52, dur:  7, sc: 1.15 },
    { color: '#6600ff', size: 370, x: 58, y: 72,  tx: -42, ty:  36, dur: 10, sc: 1.1  },
    { color: '#ffff00', size: 260, x: 48, y: 28,  tx:  36, ty:  46, dur: 12, sc: 1.25 },
  ];

  orbs.forEach(orb => {
    const el = document.createElement('div');
    el.className = 'galaxy-orb';
    el.style.cssText =
      `width:${orb.size}px;height:${orb.size}px;` +
      `left:${orb.x}%;top:${orb.y}%;` +
      `background:radial-gradient(circle,${orb.color}55,transparent 70%);` +
      `--tx:${orb.tx}px;--ty:${orb.ty}px;--dur:${orb.dur}s;--sc:${orb.sc};`;
    container.appendChild(el);
  });
}

function createStarryBg(container) {
  const frag = document.createDocumentFragment();

  for (let i = 0; i < 190; i++) {
    const star  = document.createElement('div');
    const size  = Math.random() < 0.65 ? 1 : Math.random() < 0.65 ? 2 : 3;
    const isDrift = Math.random() < 0.12;
    const minOp  = 0.15 + Math.random() * 0.3;
    const maxOp  = Math.min(minOp + 0.35 + Math.random() * 0.5, 1);
    const dur    = isDrift ? 15 + Math.random() * 20 : 2 + Math.random() * 4;

    star.className = isDrift ? 'star drifting' : 'star';
    star.style.cssText =
      `width:${size}px;height:${size}px;` +
      `left:${(Math.random() * 100).toFixed(2)}%;` +
      `top:${(Math.random() * 100).toFixed(2)}%;` +
      `--dur:${dur.toFixed(1)}s;` +
      `--min-opacity:${minOp.toFixed(2)};--max-opacity:${maxOp.toFixed(2)};` +
      `--sc:${(1 + Math.random() * 0.8).toFixed(2)};` +
      `--tx:${((Math.random() - 0.5) * 60).toFixed(1)}px;` +
      `--ty:${((Math.random() - 0.5) * 40).toFixed(1)}px;` +
      `animation-delay:-${(Math.random() * 5).toFixed(2)}s;`;
    frag.appendChild(star);
  }

  // Larger glowing blue stars
  for (let i = 0; i < 14; i++) {
    const star = document.createElement('div');
    star.className = 'star';
    star.style.cssText =
      `width:3px;height:3px;` +
      `left:${(Math.random() * 100).toFixed(2)}%;` +
      `top:${(Math.random() * 100).toFixed(2)}%;` +
      `background:#aad4ff;` +
      `box-shadow:0 0 4px 2px rgba(110,181,255,0.6);` +
      `--dur:${(3 + Math.random() * 3).toFixed(1)}s;` +
      `--min-opacity:0.40;--max-opacity:1;--sc:1.5;` +
      `animation-delay:-${(Math.random() * 5).toFixed(2)}s;`;
    frag.appendChild(star);
  }

  container.appendChild(frag);
}

// Apply saved theme on startup
applyTheme(currentTheme);

// Theme dot click handlers
document.querySelectorAll('.theme-dot').forEach(dot => {
  dot.addEventListener('click', () => applyTheme(dot.dataset.theme));
});

// ─── Settings Panel ───────────────────────────────────────────────────────────
document.getElementById('settings-btn').addEventListener('click', openSettings);
document.getElementById('settings-close-btn').addEventListener('click', closeSettings);
document.getElementById('lock-btn').addEventListener('click', handleLock);

function openSettings()  { settingsPanel.classList.remove('hidden'); }
function closeSettings() { settingsPanel.classList.add('hidden'); }

// ─── Startup toggle ───────────────────────────────────────────────────────────
const startupToggle = document.getElementById('startup-toggle');
if (startupToggle) {
  (async () => {
    try {
      startupToggle.checked = await invoke('get_startup_enabled');
    } catch {
      startupToggle.checked = false;
    }
  })();

  startupToggle.addEventListener('change', async () => {
    startupToggle.disabled = true;
    try {
      await invoke('set_startup_enabled', { enabled: startupToggle.checked });
      showToast(startupToggle.checked ? 'VaultX will launch on startup.' : 'Startup disabled.', 'info');
    } catch (err) {
      startupToggle.checked = !startupToggle.checked;
      showToast(String(err) || 'Failed to update startup setting.', 'error');
    } finally {
      startupToggle.disabled = false;
    }
  });
}

// ─── Updates (tauri-plugin-updater) ──────────────────────────────────────────
const checkUpdatesBtn    = document.getElementById('check-updates-btn');
const installUpdateBtn   = document.getElementById('install-update-btn');
const updateCurrentVersionEl = document.getElementById('update-current-version');
const updateStatusTextEl     = document.getElementById('update-status-text');
const updateProgressEl       = document.getElementById('update-progress');

let pendingUpdate = null;

// Set the real app version from Tauri
(async () => {
  try {
    const { getVersion } = window.__TAURI__.app;
    const ver = await getVersion();
    if (updateCurrentVersionEl) updateCurrentVersionEl.textContent = `Current version: v${ver}`;
    const footerEl = document.querySelector('.settings-version');
    if (footerEl) footerEl.textContent = `v${ver}`;
  } catch (_) {}
})();

async function runUpdateCheck(silent = false) {
  if (checkUpdatesBtn) checkUpdatesBtn.disabled = true;
  if (!silent && updateStatusTextEl) updateStatusTextEl.textContent = 'Checking for updates...';
  try {
    const { check } = window.__TAURI__.updater;
    const update = await check();
    if (update) {
      pendingUpdate = update;
      if (updateStatusTextEl) updateStatusTextEl.textContent = `Update available: v${update.version}`;
      if (installUpdateBtn)   installUpdateBtn.classList.remove('hidden');
      if (!silent) showToast(`Update available: v${update.version}`, 'info', 7000);
    } else {
      pendingUpdate = null;
      if (installUpdateBtn)   installUpdateBtn.classList.add('hidden');
      if (updateStatusTextEl) updateStatusTextEl.textContent = 'You are using the latest version.';
      if (!silent) showToast('You are on the latest version.', 'success');
    }
  } catch (err) {
    if (updateStatusTextEl) updateStatusTextEl.textContent = 'Failed to check for updates.';
    if (!silent) showToast(String(err) || 'Update check failed', 'error');
  } finally {
    if (checkUpdatesBtn) checkUpdatesBtn.disabled = false;
  }
}

checkUpdatesBtn?.addEventListener('click', () => runUpdateCheck(false));

installUpdateBtn?.addEventListener('click', async () => {
  if (!pendingUpdate) return;
  try {
    if (installUpdateBtn)   installUpdateBtn.disabled = true;
    if (updateStatusTextEl) updateStatusTextEl.textContent = 'Downloading update...';
    if (updateProgressEl)   updateProgressEl.classList.remove('hidden');

    let downloaded = 0;
    let total = 0;
    await pendingUpdate.downloadAndInstall((event) => {
      if (event.event === 'Started' && event.data.contentLength) {
        total = event.data.contentLength;
      } else if (event.event === 'Progress') {
        downloaded += event.data.chunkLength;
        if (total > 0 && updateProgressEl) {
          const pct = Math.round((downloaded / total) * 100);
          updateProgressEl.style.setProperty('--progress', `${pct}%`);
          updateStatusTextEl.textContent = `Downloading... ${pct}%`;
        }
      } else if (event.event === 'Finished') {
        if (updateStatusTextEl) updateStatusTextEl.textContent = 'Installing... The app will restart.';
      }
    });

    // If we get here, the app should restart automatically via the plugin.
    // Fallback message in case it doesn't:
    if (updateStatusTextEl) updateStatusTextEl.textContent = 'Update installed. Please restart the app.';
  } catch (err) {
    if (updateStatusTextEl) updateStatusTextEl.textContent = 'Update failed.';
    showToast(String(err) || 'Update install failed', 'error');
  } finally {
    if (installUpdateBtn)  installUpdateBtn.disabled = false;
    if (updateProgressEl)  updateProgressEl.classList.add('hidden');
  }
});

async function handleLock() {
  try {
    await invoke('lock');
    // Resize back to login dimensions then navigate
    const { LogicalSize } = window.__TAURI__.dpi;
    await appWindow.setResizable(false);
    await appWindow.setSize(new LogicalSize(420, 540));
    window.location.href = 'login.html';
  } catch (err) {
    showToast(String(err), 'error');
  }
}

// ─── Toast Notifications ──────────────────────────────────────────────────────
function showToast(message, type = 'info', duration = type === 'error' ? 7000 : 4000) {
  // Keep the UI tidy: replace old info toasts, cap count.
  if (type === 'info') {
    toastContainer.querySelectorAll('.toast.info').forEach(t => t.remove());
  }
  const toasts = toastContainer.querySelectorAll('.toast');
  if (toasts.length >= 3) {
    toasts[0].remove();
  }

  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  toastContainer.appendChild(toast);

  // fade-out then remove
  setTimeout(() => {
    toast.style.transition = 'opacity 0.28s';
    toast.style.opacity = '0';
    setTimeout(() => toast.remove(), 300);
  }, duration);
}

// ─── Init ─────────────────────────────────────────────────────────────────────
(async () => {
  await initDataDragonVersion();
  await renderAllCards();
  if (autoCheckUpdates) {
    runUpdateCheck(true);
  }
})();
