'use strict';

// ─── Mode ─────────────────────────────────────────────────────────────────────
// Set by Rust before the page loads. Low performance never loads an image.
const LOW = window.__VAULTX_LOW__ === true;
document.getElementById('app').appendChild(
  document.getElementById(LOW ? 'low-shell' : 'normal-shell').content.cloneNode(true)
);

// ─── Tauri IPC ────────────────────────────────────────────────────────────────
const { invoke } = window.__TAURI__.core;
const appWindow  = window.__TAURI__.window.getCurrentWindow();

document.getElementById('tb-minimize').addEventListener('click', () => appWindow.minimize());
document.getElementById('tb-maximize').addEventListener('click', () => appWindow.toggleMaximize());
document.getElementById('tb-close').addEventListener('click', () => appWindow.close());

// ─── State ────────────────────────────────────────────────────────────────────
let allAccounts = [];
let sortedAccounts = [];
let activeLobbyData = null;
let currentSortMode = localStorage.getItem('accountsSortMode') || 'highest';

// ─── DOM Refs ─────────────────────────────────────────────────────────────────
const appRoot        = document.getElementById('app');
const accountList    = document.getElementById('account-list');
const emptyState     = document.getElementById('empty-state');
const refreshAllBtn  = document.getElementById('refresh-all-btn');
const sortSelect     = document.getElementById('sort-select');
const accountModal   = document.getElementById('account-modal');
const settingsPanel  = document.getElementById('settings-panel');
const modalTitle     = document.getElementById('modal-title');
const modalDeleteBtn = document.getElementById('modal-delete-btn');
const editIdInput    = document.getElementById('edit-id');
const toastContainer = document.getElementById('toast-container');
const lobbyModal     = document.getElementById('lobby-modal');
const lobbyMeta      = document.getElementById('lobby-meta');
const lobbyList      = document.getElementById('lobby-list');
const lobbyError     = document.getElementById('lobby-error');

// ─── Rank helpers ─────────────────────────────────────────────────────────────
const TIER_COLORS = {
  IRON: '#8a8a8a', BRONZE: '#b0764a', SILVER: '#a9b3bd',
  GOLD: '#d9a13b', PLATINUM: '#4fa89e', EMERALD: '#2bb561',
  DIAMOND: '#7c8ce0', MASTER: '#b06be8', GRANDMASTER: '#e0555b',
  CHALLENGER: '#f4c874',
};

const TIER_ORDER = {
  CHALLENGER: 9, GRANDMASTER: 8, MASTER: 7, DIAMOND: 6, EMERALD: 5,
  PLATINUM: 4, GOLD: 3, SILVER: 2, BRONZE: 1, IRON: 0, UNRANKED: -1
};

const DIVISION_ORDER = { I: 4, II: 3, III: 2, IV: 1 };
const DIVISION_NUMBER = { I: '1', II: '2', III: '3', IV: '4' };

function rankScore(stats) {
  const solo = stats?.solo;
  const tierScore = TIER_ORDER[(solo?.tier || 'UNRANKED').toUpperCase()] ?? -1;
  if (tierScore < 0) return -1;
  const div = DIVISION_ORDER[(solo.division || '').toUpperCase()] ?? 0;
  return tierScore * 1000 + div * 100 + Number(solo.lp || 0);
}

function capitalise(str) {
  return str ? str.charAt(0).toUpperCase() + str.slice(1).toLowerCase() : '';
}

function isRanked(rank) {
  return !!rank && !!rank.tier && rank.tier !== 'UNRANKED';
}

function tierColor(rank) {
  return isRanked(rank) ? (TIER_COLORS[rank.tier.toUpperCase()] || 'var(--faint)') : 'var(--faint)';
}

function tierName(rank) {
  if (!isRanked(rank)) return 'Unranked';
  const div = DIVISION_NUMBER[rank.division] ? ` ${DIVISION_NUMBER[rank.division]}` : '';
  return `${capitalise(rank.tier)}${div}`;
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

function buildOpggUrl(riotId, region) {
  const name = riotId.replace('#', '-');
  const r = OPGG_REGIONS[region] || region.toLowerCase();
  return `https://www.op.gg/summoners/${r}/${encodeURIComponent(name)}`;
}

function lobbyRiotIds(players) {
  return players.map(p => `${p.gameName}#${p.tagLine}`).join(',');
}

function buildOpggMultiUrl(players, region) {
  const r = (region || 'na').toLowerCase();
  return `https://www.op.gg/multisearch/${r}?summoners=${encodeURIComponent(lobbyRiotIds(players))}`;
}

function buildTrackerMultiUrl(players, region) {
  const r = (region || 'na').toLowerCase();
  return `https://tracker.gg/lol/multisearch/${r}/${encodeURIComponent(lobbyRiotIds(players))}`;
}

// ─── Icons ────────────────────────────────────────────────────────────────────
const icon = (body) => `<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;
const ICONS = {
  user:    icon('<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-7 8-7s8 3 8 7"/>'),
  key:     icon('<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3"/>'),
  refresh: icon('<path d="M20 11a8 8 0 0 0-14.3-4.3L3 9.500M3 4v5.500h5.500M4 13a8 8 0 0 0 14.3 4.3L21 14.500M21 20v-5.500h-5.500"/>'),
  edit:    icon('<path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16v4z"/>'),
  check:   icon('<path d="M5 12.500l4.500 4.500L19 7.500"/>'),
  play:    '<svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.500v15a1 1 0 0 0 1.500.860l12.500-7.500a1 1 0 0 0 0-1.720L8.500 3.640A1 1 0 0 0 7 4.500z"/></svg>',
};

// ─── Low performance: list rows ───────────────────────────────────────────────
function buildRow(account) {
  const solo = account.stats?.solo;
  const id = esc(account.id);
  const updated = account.stats?.fetchedAt
    ? `Updated ${new Date(account.stats.fetchedAt).toLocaleString()}`
    : 'No stats yet';
  const rank = isRanked(solo)
    ? `<span class="rank"><span class="tier-dot" style="background:${tierColor(solo)}"></span>${esc(tierName(solo))}</span>
       <span class="sub" style="padding-left:14px">${esc(solo.lp)} LP</span>`
    : '<span class="rank dim"><span class="tier-dot"></span>Unranked</span>';
  const wr = isRanked(solo) && solo.winRate != null
    ? `<span class="${parseFloat(solo.winRate) >= 50 ? 'good' : 'bad'}">${esc(Math.round(solo.winRate))}%</span>
       <span class="sub">${esc(solo.wins)}W ${esc(solo.losses)}L</span>`
    : '';

  const row = document.createElement('div');
  row.className = 'row';
  row.dataset.row = account.id;
  row.innerHTML = `
    <div class="cell">
      <span class="name" title="${esc(account.label)}">${esc(account.label)}</span>
      <span class="sub-line">
        <button class="link js-opgg" data-url="${esc(buildOpggUrl(account.riotId, account.region))}" title="Open on op.gg">${esc(account.riotId)}</button>
        <span class="region">${esc(account.region)}</span>
      </span>
    </div>
    <div class="cell" title="${esc(updated)}">${rank}</div>
    <div class="cell">${wr}</div>
    <div class="actions">
      <button class="icon-btn quiet js-copy-user" data-id="${id}" title="Copy username">${ICONS.user}</button>
      <button class="icon-btn quiet js-copy-pw" data-id="${id}" title="Copy password">${ICONS.key}</button>
      <button class="icon-btn quiet js-refresh" data-id="${id}" title="Refresh stats">${ICONS.refresh}</button>
      <button class="icon-btn quiet js-edit" data-id="${id}" title="Edit or delete">${ICONS.edit}</button>
      <button class="play js-launch" data-id="${id}" title="Open Riot Client">${ICONS.play}</button>
    </div>`;
  return row;
}

// ─── Normal: sidebar + detail ─────────────────────────────────────────────────
const DD_FALLBACK_VERSION = '16.5.1';
let ddVersion = DD_FALLBACK_VERSION;
const DD_VERSION_CACHE_KEY = 'ddragonVersionCacheV1';
const DD_VERSION_CACHE_MAX_AGE_MS = 24 * 60 * 60 * 1000;

async function initDataDragonVersion() {
  try {
    const cached = JSON.parse(localStorage.getItem(DD_VERSION_CACHE_KEY) || 'null');
    if (typeof cached?.version === 'string' && Date.now() - (cached.ts || 0) < DD_VERSION_CACHE_MAX_AGE_MS) {
      ddVersion = cached.version;
      return;
    }
    const res = await fetch('https://ddragon.leagueoflegends.com/api/versions.json', { cache: 'no-store' });
    if (!res.ok) return;
    const latest = (await res.json())[0];
    if (typeof latest === 'string') {
      ddVersion = latest;
      localStorage.setItem(DD_VERSION_CACHE_KEY, JSON.stringify({ version: latest, ts: Date.now() }));
    }
  } catch {
    // Keep the fallback version; icons may just be a patch behind.
  }
}

// Champions whose Data Dragon key differs from the display name
const DD_OVERRIDES = {
  "Wukong": "MonkeyKing", "Renata Glasc": "Renata", "Nunu & Willump": "Nunu",
  "LeBlanc": "Leblanc", "Dr. Mundo": "DrMundo", "Jarvan IV": "JarvanIV",
  "Kai'Sa": "Kaisa", "Kha'Zix": "Khazix", "Cho'Gath": "Chogath",
  "Kog'Maw": "KogMaw", "Vel'Koz": "Velkoz", "Bel'Veth": "Belveth",
  "Rek'Sai": "RekSai", "K'Sante": "KSante", "Tahm Kench": "TahmKench",
  "Aurelion Sol": "AurelionSol", "Lee Sin": "LeeSin", "Master Yi": "MasterYi",
  "Miss Fortune": "MissFortune", "Twisted Fate": "TwistedFate", "Xin Zhao": "XinZhao",
};

function champIconUrl(name) {
  const key = DD_OVERRIDES[name] || name.split(/\s+/)
    .map(w => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')
    .replace(/[^a-zA-Z0-9]/g, '');
  return `https://ddragon.leagueoflegends.com/cdn/${ddVersion}/img/champion/${key}.png`;
}

function avatarHTML(account, size) {
  const initial = esc((account.label || '?').charAt(0).toUpperCase());
  const iconUrl = account.stats?.iconUrl;
  const img = iconUrl ? `<img src="${esc(iconUrl)}" alt="">` : '';
  const color = tierColor(account.stats?.solo);
  const aura = isRanked(account.stats?.solo) ? ` has-aura` : '';
  return `<span class="avatar ${size}${aura}" style="border-color:${color};--aura:${color}">${initial}${img}</span>`;
}

const ROLE_NAMES = { TOP: 'Top', JUNGLE: 'Jungle', MIDDLE: 'Mid', BOTTOM: 'Bot', UTILITY: 'Support' };

function fixed(value, digits = 1) {
  return Number(value || 0).toFixed(digits);
}

function wrClass(pct) {
  return pct >= 50 ? 'good' : 'bad';
}

function queueHTML(title, rank) {
  const ranked = isRanked(rank);
  const total = ranked ? rank.wins + rank.losses : 0;
  const pct = total > 0 ? Math.round(rank.wins / total * 100) : null;
  return `<div class="queue">
    <span class="stat-label">${title}</span>
    <span class="stat-value" style="color:${ranked ? tierColor(rank) : 'var(--dim)'}">${esc(tierName(rank))}</span>
    ${ranked ? `<span class="queue-line"><span>${esc(rank.lp)} LP · ${esc(rank.wins)}W ${esc(rank.losses)}L</span>${pct != null ? `<span class="${wrClass(pct)}">${pct}%</span>` : ''}</span>
      <span class="wr-track"><span class="wr-fill" style="width:${pct ?? 0}%"></span></span>` : ''}
  </div>`;
}

function metricHTML(label, value, title) {
  return `<div class="metric" title="${esc(title)}">
    <span class="metric-value">${esc(value)}</span>
    <span class="stat-label">${label}</span>
  </div>`;
}

function recentHTML(recent) {
  if (!recent) return '';
  const pct = recent.games > 0 ? Math.round(recent.wins / recent.games * 100) : 0;

  const matches = recent.matches || [];
  const latestWin = matches[0]?.win;
  let streak = 0;
  while (streak < matches.length && matches[streak].win === latestWin) streak++;
  const streakText = streak >= 2 ? `${latestWin ? 'W' : 'L'}${streak}` : '';

  // Stored newest first; drawn oldest to newest, so the latest game is on the right.
  const games = [...matches].reverse().map(m => `
    <span class="game ${m.win ? 'is-win' : 'is-loss'}" title="${esc(m.champion)}: ${m.win ? 'Win' : 'Loss'}, ${esc(m.kills)}/${esc(m.deaths)}/${esc(m.assists)}">
      ${esc(m.champion.slice(0, 2))}<img src="${esc(champIconUrl(m.champion))}" alt="">
    </span>`).join('');
  const role = recent.mainRole && ROLE_NAMES[recent.mainRole]
    ? `${ROLE_NAMES[recent.mainRole]} ${recent.mainRoleGames}/${recent.games}`
    : '';

  return `<div class="recent">
    <div class="recent-head">
      <span class="stat-label">Last ${esc(recent.games)} ranked${role ? ` · ${esc(role)}` : ''}</span>
      <span class="recent-record">${streakText ? `<span class="streak ${latestWin ? 'good' : 'bad'}" title="Current ${latestWin ? 'win' : 'loss'} streak">${streakText}</span> ` : ''}${esc(recent.wins)}W ${esc(recent.losses)}L <span class="${wrClass(pct)}">${pct}%</span></span>
    </div>
    <div class="games" aria-label="Last ${esc(recent.games)} ranked games, oldest to newest: ${esc(recent.wins)} wins, ${esc(recent.losses)} losses">${games}</div>
    <div class="metrics">
      ${metricHTML('KDA', fixed(recent.kda), `${fixed(recent.kills)} / ${fixed(recent.deaths)} / ${fixed(recent.assists)} average kills / deaths / assists`)}
      ${metricHTML('CS/min', fixed(recent.csPerMin), 'Creep score per minute')}
      ${metricHTML('Dmg/min', Math.round(recent.damagePerMin), 'Damage to champions per minute')}
      ${metricHTML('Vision/min', fixed(recent.visionPerMin, 2), 'Vision score per minute')}
    </div>
  </div>`;
}

function champRowHTML(c) {
  const wr = c.winRate != null ? Math.round(c.winRate) : null;
  return `<div class="champ" title="${esc(c.name)}">
    <span class="champ-icon">${esc(c.name.slice(0, 2))}<img src="${esc(champIconUrl(c.name))}" alt=""></span>
    <span class="champ-name">${esc(c.name)}</span>
    <span class="champ-cell">${esc(c.games)}g</span>
    <span class="champ-cell">${c.kda != null ? `${fixed(c.kda)} KDA` : ''}</span>
    <span class="champ-cell ${wr != null ? wrClass(wr) : ''}">${wr != null ? `${wr}%` : '-'}</span>
  </div>`;
}

function buildCard(account) {
  const stats = account.stats;
  const solo = stats?.solo;
  const id = esc(account.id);
  const champs = (stats?.topChampions || []).slice(0, 3);
  const updated = stats?.fetchedAt
    ? `Updated ${new Date(stats.fetchedAt).toLocaleString()}`
    : 'No stats yet';

  const card = document.createElement('article');
  card.className = 'card';
  card.dataset.row = account.id;
  card.style.setProperty('--stripe', isRanked(solo) ? tierColor(solo) : 'rgba(255, 255, 255, 0.55)');
  card.innerHTML = `
    <div class="card-stripes"></div>
    <div class="card-head">
      ${avatarHTML(account, 'medium')}
      <div class="card-id">
        <span class="card-name" title="${esc(account.label)}">${esc(account.label)}</span>
        <span class="sub-line">
          <button class="link js-opgg" data-url="${esc(buildOpggUrl(account.riotId, account.region))}" title="Open on op.gg">${esc(account.riotId)}</button>
          <span class="region">${esc(account.region)}</span>
        </span>
        ${stats?.summonerLevel != null ? `<span class="stat-sub">Level ${esc(stats.summonerLevel)}</span>` : ''}
      </div>
      <button class="play card-play js-launch" data-id="${id}" aria-label="Play ${esc(account.label)}" title="Open Riot Client">${ICONS.play}</button>
    </div>

    <div class="queues" title="${esc(updated)}">
      ${queueHTML('Solo / Duo', solo)}
      ${queueHTML('Flex', stats?.flex)}
    </div>

    ${recentHTML(stats?.recent)}

    ${champs.length
      ? `<div class="card-champs">${champs.map(champRowHTML).join('')}</div>`
      : ''}

    ${!stats?.recent ? `<span class="hint">${stats ? 'Refresh for recent-game analytics.' : 'No stats yet. Refresh to fetch them.'}</span>` : ''}

    <div class="card-foot">
      <button class="card-action js-copy-user" data-id="${id}" title="Copy username">${ICONS.user}<span>Username</span></button>
      <button class="card-action js-copy-pw" data-id="${id}" title="Copy password">${ICONS.key}<span>Password</span></button>
      <span class="toolbar-spacer"></span>
      <button class="icon-btn js-refresh" data-id="${id}" aria-label="Refresh stats" title="Refresh stats">${ICONS.refresh}</button>
      <button class="icon-btn js-edit" data-id="${id}" aria-label="Edit or delete" title="Edit or delete">${ICONS.edit}</button>
    </div>`;
  return card;
}

// ─── Render ───────────────────────────────────────────────────────────────────
const buildAccount = LOW ? buildRow : buildCard;

function renderList() {
  accountList.replaceChildren(...sortedAccounts.map(buildAccount));
  emptyState.classList.toggle('hidden', allAccounts.length > 0);
}

function sortAccounts() {
  sortedAccounts = [...allAccounts];
  if (currentSortMode === 'highest') {
    sortedAccounts.sort((a, b) => rankScore(b.stats) - rankScore(a.stats));
  } else if (currentSortMode === 'lowest') {
    sortedAccounts.sort((a, b) => rankScore(a.stats) - rankScore(b.stats));
  } else if (currentSortMode === 'region') {
    sortedAccounts.sort((a, b) => (a.region || '').localeCompare(b.region || '') || a.label.localeCompare(b.label));
  }
}

async function renderAccounts() {
  try {
    allAccounts = await invoke('get_all');
  } catch (err) {
    return showToast(String(err), 'error');
  }
  const countEl = document.getElementById('account-count');
  if (countEl) countEl.textContent = allAccounts.length || '';
  sortAccounts();
  renderList();
}

// ─── Account actions (event delegation) ───────────────────────────────────────
// A failed image is removed, revealing the letter behind it. This has to be a
// listener: the app's security policy blocks inline onerror attributes.
appRoot.addEventListener('error', (e) => {
  if (e.target.tagName === 'IMG') e.target.remove();
}, true);

appRoot.addEventListener('click', async (e) => {
  const profileBtn = e.target.closest('.js-opgg');
  if (profileBtn) {
    invoke('open_external', { url: profileBtn.dataset.url }).catch(() => {});
    return;
  }

  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  const id = btn.dataset.id;

  if (btn.classList.contains('js-launch')) {
    await handleLaunchAccount(id, btn);
  } else if (btn.classList.contains('js-edit')) {
    openEditModal(id);
  } else if (btn.classList.contains('js-refresh')) {
    const result = await refreshAccount(id);
    if (!result.ok) showToast(result.error || 'Failed to fetch stats', 'error');
  } else if (btn.classList.contains('js-copy-user')) {
    const acc = allAccounts.find(a => a.id === id);
    if (!acc) return;
    try {
      await invoke('write_text', { text: acc.username });
      showCopied(btn);
    } catch (err) {
      showToast(String(err), 'error');
    }
  } else if (btn.classList.contains('js-copy-pw')) {
    try {
      await invoke('copy_password', { id });
      showCopied(btn);
    } catch (err) {
      showToast(String(err) || 'Failed to copy', 'error');
    }
  }
});

function showCopied(btn) {
  const original = btn.innerHTML;
  btn.innerHTML = LOW ? ICONS.check : `${ICONS.check}<span>Copied</span>`;
  btn.classList.add('copied');
  setTimeout(() => {
    btn.innerHTML = original;
    btn.classList.remove('copied');
  }, 1000);
}

async function handleLaunchAccount(id, btn) {
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;
  btn.disabled = true;
  try {
    await invoke('launch_account', { id });
    showToast(`Riot Client opened for ${acc.label}.`, 'success');
  } catch (err) {
    showToast(String(err) || 'Launch failed', 'error');
  } finally {
    btn.disabled = false;
  }
}

/** Refreshes one account and redraws it in place. Never throws. */
async function refreshAccount(id) {
  const row = accountList.querySelector(`[data-row="${id}"]`);
  const btn = row?.querySelector('.js-refresh');
  if (btn) { btn.disabled = true; btn.classList.add('busy'); }

  try {
    const stats = await invoke('refresh_stats', { id });
    const acc = allAccounts.find(a => a.id === id);
    if (acc) {
      acc.stats = stats;
      accountList.querySelector(`[data-row="${id}"]`)?.replaceWith(buildAccount(acc));
    }
    return { ok: true };
  } catch (err) {
    if (btn) { btn.disabled = false; btn.classList.remove('busy'); }
    return { ok: false, error: String(err) };
  }
}

// ─── Delete ───────────────────────────────────────────────────────────────────
const deleteModal = document.getElementById('delete-modal');
let deleteResolve = null;

document.getElementById('delete-cancel-btn').addEventListener('click', () => { deleteModal.classList.add('hidden'); deleteResolve?.(false); });
document.getElementById('delete-confirm-btn').addEventListener('click', () => { deleteModal.classList.add('hidden'); deleteResolve?.(true); });

function confirmDelete(label) {
  return new Promise(resolve => {
    deleteResolve = resolve;
    document.getElementById('delete-modal-msg').textContent = `Delete "${label}" from the vault? This cannot be undone.`;
    deleteModal.classList.remove('hidden');
  });
}

modalDeleteBtn.addEventListener('click', async () => {
  const acc = allAccounts.find(a => a.id === editIdInput.value);
  if (!acc || !await confirmDelete(acc.label)) return;
  try {
    await invoke('delete_account', { id: acc.id });
    closeModal();
    await renderAccounts();
    showToast(`${acc.label} deleted.`);
  } catch (err) {
    showToast(String(err) || 'Failed to delete', 'error');
  }
});

// ─── Lobby ────────────────────────────────────────────────────────────────────
document.getElementById('lobby-done-btn').addEventListener('click', closeLobbyViewer);
document.getElementById('lobby-refresh-btn').addEventListener('click', fetchLobby);
document.getElementById('lobby-open-opgg-btn').addEventListener('click', () => openLobbyMultiSite('opgg'));
document.getElementById('lobby-open-tracker-btn').addEventListener('click', () => openLobbyMultiSite('tracker'));
document.getElementById('lobby-viewer-btn').addEventListener('click', async () => {
  lobbyModal.classList.remove('hidden');
  await fetchLobby();
});

lobbyModal.addEventListener('click', (e) => {
  if (e.target === lobbyModal) closeLobbyViewer();
});

function closeLobbyViewer() {
  lobbyModal.classList.add('hidden');
  activeLobbyData = null;
}

function renderLobbyPlayers(players) {
  if (!players || players.length === 0) {
    lobbyList.innerHTML = '<div class="hint">No players yet. Open this during champ select.</div>';
    return;
  }
  lobbyList.innerHTML = players.map(p => {
    const name = (p.gameName && p.tagLine) ? `${p.gameName}#${p.tagLine}` : (p.summonerName || 'Hidden');
    return `<div>${esc(name)}</div>`;
  }).join('');
}

async function fetchLobby() {
  const refreshBtn = document.getElementById('lobby-refresh-btn');
  refreshBtn.disabled = true;
  lobbyError.classList.add('hidden');
  lobbyMeta.textContent = 'Checking League client...';
  lobbyList.innerHTML = '';

  try {
    const data = await invoke('get_lobby_view');
    activeLobbyData = data;
    lobbyMeta.textContent = `${data.phase} · ${data.region || 'NA'} · ${data.players.length} players`;
    renderLobbyPlayers(data.players);
  } catch (err) {
    activeLobbyData = null;
    lobbyMeta.textContent = '';
    lobbyError.textContent = String(err) || 'Failed to load lobby.';
    lobbyError.classList.remove('hidden');
  } finally {
    refreshBtn.disabled = false;
  }
}

async function openLobbyMultiSite(site) {
  const eligible = (activeLobbyData?.players || []).filter(p => p.gameName && p.tagLine);
  if (eligible.length === 0) {
    showToast('No lobby players with Riot IDs yet.');
    return;
  }
  const region = activeLobbyData.region || 'NA';
  const url = site === 'tracker' ? buildTrackerMultiUrl(eligible, region) : buildOpggMultiUrl(eligible, region);
  try {
    await invoke('open_external', { url });
  } catch (err) {
    showToast(String(err) || 'Failed to open browser.', 'error');
  }
}

// ─── Auto accept ──────────────────────────────────────────────────────────────
const autoAcceptBtn = document.getElementById('auto-accept-header-btn');

function showAutoAccept(enabled) {
  autoAcceptBtn.classList.toggle('on', enabled);
  if (!LOW) autoAcceptBtn.setAttribute('aria-checked', String(enabled));
}

invoke('get_auto_accept_status')
  .then(showAutoAccept)
  .catch(() => {});

autoAcceptBtn.addEventListener('click', async () => {
  const next = !autoAcceptBtn.classList.contains('on');
  try {
    showAutoAccept(await invoke('set_auto_accept_enabled', { enabled: next }));
  } catch (err) {
    showToast(String(err) || 'Failed to update auto-accept.', 'error');
  }
});

// ─── Sort ─────────────────────────────────────────────────────────────────────
sortSelect.value = currentSortMode;
sortSelect.addEventListener('change', async () => {
  currentSortMode = sortSelect.value;
  localStorage.setItem('accountsSortMode', currentSortMode);
  await renderAccounts();
});

// ─── Refresh all ──────────────────────────────────────────────────────────────
// Runs once on opening and whenever the user asks. Sitting idle never triggers it:
// after a while the app only offers a refresh and waits for an answer.
const STALE_PROMPT_MS = 60 * 60 * 1000;
let refreshingAll = false;
let lastRefreshAt = Date.now();
let staleTimer = null;

refreshAllBtn.addEventListener('click', refreshAllAccounts);

async function refreshAllAccounts() {
  if (refreshingAll || allAccounts.length === 0) return;
  refreshingAll = true;
  hideStalePrompt();
  refreshAllBtn.disabled = true;
  if (LOW) refreshAllBtn.textContent = 'Refreshing...';
  else refreshAllBtn.classList.add('busy');

  const ids = sortedAccounts.map(a => a.id);
  let failed = 0;
  for (let i = 0; i < ids.length; i++) {
    setProgressToast(`Refreshing ${i + 1} of ${ids.length}...`);
    const result = await refreshAccount(ids[i]);
    if (!result.ok) failed++;
  }

  setProgressToast(null);
  sortAccounts();
  renderList();
  refreshingAll = false;
  refreshAllBtn.disabled = false;
  if (LOW) refreshAllBtn.textContent = 'Refresh all';
  else refreshAllBtn.classList.remove('busy');

  const ok = ids.length - failed;
  if (failed === 0)  showToast(`${ok} updated.`, 'success');
  else if (ok === 0) showToast('All refreshes failed. Check the Riot IDs and regions.', 'error');
  else               showToast(`${ok} updated, ${failed} failed.`);

  lastRefreshAt = Date.now();
  scheduleStalePrompt(STALE_PROMPT_MS);
}

function scheduleStalePrompt(delay) {
  clearTimeout(staleTimer);
  staleTimer = setTimeout(() => {
    showStalePrompt();
    scheduleStalePrompt(STALE_PROMPT_MS); // Keeps the hours current, or asks again after "Not now".
  }, delay);
}

function showStalePrompt() {
  if (refreshingAll || allAccounts.length === 0) return;
  const hours = Math.max(1, Math.floor((Date.now() - lastRefreshAt) / (60 * 60 * 1000)));
  let prompt = document.getElementById('stale-prompt');
  if (!prompt) {
    prompt = document.createElement('div');
    prompt.id = 'stale-prompt';
    prompt.className = 'toast prompt';
    prompt.innerHTML = `
      <span id="stale-text"></span>
      <button class="btn primary" id="stale-refresh-btn">Refresh</button>
      <button class="btn" id="stale-dismiss-btn">Not now</button>`;
    toastContainer.appendChild(prompt);
    document.getElementById('stale-refresh-btn').addEventListener('click', refreshAllAccounts);
    document.getElementById('stale-dismiss-btn').addEventListener('click', hideStalePrompt);
  }
  document.getElementById('stale-text').textContent = `Stats are ${hours} ${hours === 1 ? 'hour' : 'hours'} outdated.`;
}

function hideStalePrompt() {
  document.getElementById('stale-prompt')?.remove();
}

// ─── Add / Edit ───────────────────────────────────────────────────────────────
document.getElementById('add-account-btn').addEventListener('click', openAddModal);
document.getElementById('modal-cancel-btn').addEventListener('click', closeModal);
document.getElementById('modal-save-btn').addEventListener('click', handleModalSave);

function openAddModal() {
  modalTitle.textContent = 'Add account';
  editIdInput.value = '';
  clearModalFields();
  modalDeleteBtn.classList.add('hidden');
  document.getElementById('field-password').placeholder = '';
  accountModal.classList.remove('hidden');
  document.getElementById('field-label').focus();
}

function openEditModal(id) {
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;

  modalTitle.textContent = 'Edit account';
  clearModalFields();
  editIdInput.value = id;
  document.getElementById('field-label').value    = acc.label;
  document.getElementById('field-username').value = acc.username;
  document.getElementById('field-riotid').value   = acc.riotId;
  document.getElementById('field-region').value   = acc.region;
  document.getElementById('field-password').placeholder = 'Leave blank to keep current';
  modalDeleteBtn.classList.remove('hidden');
  accountModal.classList.remove('hidden');
  document.getElementById('field-label').focus();
}

function closeModal() {
  accountModal.classList.add('hidden');
  clearModalFields();
}

function clearModalFields() {
  ['field-label', 'field-username', 'field-password', 'field-riotid'].forEach(id => {
    document.getElementById(id).value = '';
  });
  document.getElementById('field-region').value = 'NA';
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

  if (!label)    return showModalError('Label is required.');
  if (!username) return showModalError('Username is required.');
  if (!riotId)   return showModalError('Riot ID is required.');
  if (!/^.+#.+$/.test(riotId)) return showModalError('Riot ID must look like GameName#TAG.');
  if (!id && !password) return showModalError('Password is required.');

  const saveBtn = document.getElementById('modal-save-btn');
  saveBtn.disabled = true;

  try {
    if (id) {
      const updates = { label, username, riotId, region };
      if (password) updates.password = password;
      await invoke('update_account', { id, updates });
      closeModal();
      await renderAccounts();
    } else {
      const newId = await invoke('add_account', { account: { label, username, password, riotId, region } });
      closeModal();
      await renderAccounts();
      refreshAccount(newId).then(result => {
        if (!result.ok) showToast(result.error || 'Failed to fetch stats', 'error');
      });
    }
  } catch (err) {
    showModalError(String(err) || 'Failed to save.');
  } finally {
    saveBtn.disabled = false;
  }
}

// ─── Settings ─────────────────────────────────────────────────────────────────
document.getElementById('settings-btn').addEventListener('click', () => {
  pickMode(LOW);
  settingsPanel.classList.remove('hidden');
});
document.getElementById('settings-close-btn').addEventListener('click', () => settingsPanel.classList.add('hidden'));
document.getElementById('lock-btn').addEventListener('click', handleLock);

const MODE_LINES = {
  normal: ['Full account view', 'Profile and champion images', 'Hover effects'],
  low: ['Compact text list', 'No profile or champion images', 'Hardware acceleration off, lowest RAM'],
};
let pickedLow = LOW;

function pickMode(low) {
  pickedLow = low;
  document.getElementById('mode-normal').classList.toggle('active', !low);
  document.getElementById('mode-low').classList.toggle('active', low);
  document.getElementById('mode-lines').innerHTML = MODE_LINES[low ? 'low' : 'normal'].map(l => `<span>${l}</span>`).join('');
  document.getElementById('mode-restart-row').classList.toggle('hidden', low === LOW);
}

document.getElementById('mode-normal').addEventListener('click', () => pickMode(false));
document.getElementById('mode-low').addEventListener('click', () => pickMode(true));
document.getElementById('mode-restart-btn').addEventListener('click', async () => {
  try {
    await invoke('set_ui_mode', { low: pickedLow });
  } catch (err) {
    showToast(String(err) || 'Failed to switch mode.', 'error');
  }
});

const startupToggle = document.getElementById('startup-toggle');
invoke('get_startup_enabled')
  .then(enabled => { startupToggle.checked = enabled; })
  .catch(() => {});

startupToggle.addEventListener('change', async () => {
  startupToggle.disabled = true;
  try {
    await invoke('set_startup_enabled', { enabled: startupToggle.checked });
  } catch (err) {
    startupToggle.checked = !startupToggle.checked;
    showToast(String(err) || 'Failed to update startup setting.', 'error');
  } finally {
    startupToggle.disabled = false;
  }
});

// ─── Updates (tauri-plugin-updater) ───────────────────────────────────────────
const checkUpdatesBtn  = document.getElementById('check-updates-btn');
const installUpdateBtn = document.getElementById('install-update-btn');
const updateStatusEl   = document.getElementById('update-status-text');

const autoCheckUpdates = localStorage.getItem('autoCheckUpdates') !== 'false';
const AUTO_UPDATE_INTERVAL_MS = 6 * 60 * 60 * 1000; // for sessions left open

let pendingUpdate = null;

window.__TAURI__.app.getVersion()
  .then(ver => { document.getElementById('update-current-version').textContent = `Version ${ver}`; })
  .catch(() => {});

async function runUpdateCheck(silent = false) {
  checkUpdatesBtn.disabled = true;
  if (!silent) updateStatusEl.textContent = 'Checking for updates...';
  try {
    const update = await window.__TAURI__.updater.check();
    pendingUpdate = update || null;
    installUpdateBtn.classList.toggle('hidden', !update);
    updateStatusEl.textContent = update ? `Update available: ${update.version}` : 'You are on the latest version.';
  } catch (err) {
    updateStatusEl.textContent = 'Failed to check for updates.';
    if (!silent) showToast(String(err) || 'Update check failed', 'error');
  } finally {
    checkUpdatesBtn.disabled = false;
  }
}

checkUpdatesBtn.addEventListener('click', () => runUpdateCheck(false));
installUpdateBtn.addEventListener('click', installPendingUpdate);

async function runAutoUpdate() {
  await runUpdateCheck(true);
  if (pendingUpdate) {
    showToast(`Updating VaultX to ${pendingUpdate.version}...`);
    await installPendingUpdate();
  }
}

async function installPendingUpdate() {
  if (!pendingUpdate) return;
  installUpdateBtn.disabled = true;
  updateStatusEl.textContent = 'Downloading update...';
  try {
    await pendingUpdate.downloadAndInstall();
    // The installer normally restarts the app before this line runs.
    updateStatusEl.textContent = 'Update installed. Please restart the app.';
  } catch (err) {
    updateStatusEl.textContent = 'Update failed.';
    showToast(String(err) || 'Update install failed', 'error');
  } finally {
    installUpdateBtn.disabled = false;
  }
}

// ─── Lock ─────────────────────────────────────────────────────────────────────
async function handleLock() {
  try {
    await invoke('lock');
    const { LogicalSize } = window.__TAURI__.dpi;
    if (await appWindow.isMaximized()) await appWindow.toggleMaximize();
    await appWindow.setResizable(false);
    await appWindow.setMinSize(null);
    await appWindow.setSize(new LogicalSize(360, 400));
    window.location.href = 'login.html';
  } catch (err) {
    showToast(String(err), 'error');
  }
}

// ─── Toasts ───────────────────────────────────────────────────────────────────
/** A toast that stays until replaced; null removes it. */
function setProgressToast(message) {
  let toast = document.getElementById('progress-toast');
  if (message == null) { toast?.remove(); return; }
  if (!toast) {
    toast = document.createElement('div');
    toast.id = 'progress-toast';
    toast.className = 'toast';
    toastContainer.appendChild(toast);
  }
  toast.textContent = message;
}

function showToast(message, type = 'info', duration = type === 'error' ? 6000 : 3000) {
  const toasts = toastContainer.querySelectorAll('.toast:not(#progress-toast):not(#stale-prompt)');
  if (toasts.length >= 3) toasts[0].remove();

  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  toastContainer.appendChild(toast);
  setTimeout(() => toast.remove(), duration);
}

// ─── Init ─────────────────────────────────────────────────────────────────────
(async () => {
  if (!LOW) await initDataDragonVersion();
  await renderAccounts();
  refreshAllAccounts();
  if (autoCheckUpdates) {
    runAutoUpdate();
    setInterval(runAutoUpdate, AUTO_UPDATE_INTERVAL_MS);
  }
})();
