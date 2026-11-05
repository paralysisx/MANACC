'use strict';

// ─── Theme ────────────────────────────────────────────────────────────────────
// Four ready-made palettes. Applied before anything is drawn.
const THEMES = ['midnight', 'violet', 'galaxy', 'starry'];
function applyTheme(theme) {
  const chosen = THEMES.includes(theme) ? theme : 'midnight';
  if (chosen === 'midnight') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = chosen;
  return chosen;
}
let currentTheme = applyTheme(localStorage.getItem('theme'));
if (window.__VAULTX_LOW__ === true) document.documentElement.dataset.low = '';

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
/** What the search box holds, lower-cased. Empty shows every account. */
let searchQuery = '';

// ─── DOM Refs ─────────────────────────────────────────────────────────────────
const appRoot        = document.getElementById('app');
const accountList    = document.getElementById('account-list');
const emptyState     = document.getElementById('empty-state');
const noMatch        = document.getElementById('no-match');
const searchBox      = document.getElementById('search-box');
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

// Shown wherever something is loading. Only ever on screen during the wait.
const LOADER = '<span class="loader"></span>';

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

const UGG_SERVERS = {
  NA: 'na1', EUW: 'euw1', EUNE: 'eun1', KR: 'kr', JP: 'jp1',
  BR: 'br1', LAN: 'la1', LAS: 'la2', OCE: 'oc1', TR: 'tr1', RU: 'ru'
};

function buildUggUrl(riotId, region) {
  const name = riotId.replace('#', '-');
  const s = UGG_SERVERS[region] || `${region.toLowerCase()}1`;
  return `https://u.gg/lol/profile/${s}/${encodeURIComponent(name)}/overview`;
}

/** The op.gg and u.gg buttons for an account. They open in the browser. */
function profileLinksHTML(account) {
  const site = (name, icon, url) =>
    `<button class="site ${icon} js-profile" data-url="${esc(url)}" title="Open on ${name}" aria-label="Open on ${name}"><img src="assets/${icon}.png" alt="" draggable="false"></button>`;
  return `<span class="sites">${site('op.gg', 'opgg', buildOpggUrl(account.riotId, account.region))}${site('u.gg', 'ugg', buildUggUrl(account.riotId, account.region))}</span>`;
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
  eye:     icon('<path d="M2 12s3.600-7 10-7 10 7 10 7-3.600 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>'),
  eyeOff:  icon('<path d="M3 3l18 18M10.500 5.200A9.800 9.800 0 0 1 12 5c6.400 0 10 7 10 7a17.500 17.500 0 0 1-3.300 4.100M6.600 6.600C3.700 8.500 2 12 2 12s3.600 7 10 7a9.600 9.600 0 0 0 4.300-1"/>'),
  pin:     icon('<path d="M9 3h6l-1 7 3 3H7l3-3-1-7zM12 13v8"/>'),
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
      <span class="name" title="${esc(account.label)}">${account.stats?.live ? '<span class="live-dot" title="In a game at the last refresh"></span>' : ''}${account.pinned ? '<span class="pin-mark" title="Pinned">●</span>' : ''}${esc(account.label)}</span>
      <span class="sub-line">
        <button class="link js-opgg" data-url="${esc(buildOpggUrl(account.riotId, account.region))}" title="Open on op.gg">${esc(account.riotId)}</button>
        <span class="region">${esc(account.region)}</span>
        ${profileLinksHTML(account)}
      </span>
    </div>
    <div class="cell" title="${esc(updated)}">${rank}</div>
    <div class="cell">${wr}</div>
    <div class="actions">
      <button class="icon-btn quiet js-copy-user" data-id="${id}" title="Copy username">${ICONS.user}</button>
      <button class="icon-btn quiet js-copy-pw" data-id="${id}" title="Copy password">${ICONS.key}</button>
      <button class="icon-btn quiet js-refresh" data-id="${id}" title="Refresh stats">${ICONS.refresh}</button>
      <button class="icon-btn quiet js-edit" data-id="${id}" title="Edit or delete">${ICONS.edit}</button>
      <button class="play js-launch" data-id="${id}" title="${playTitle(id)}">${ICONS.play}</button>
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

function champKey(name) {
  return DD_OVERRIDES[name] || name.split(/\s+/)
    .map(w => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')
    .replace(/[^a-zA-Z0-9]/g, '');
}

function champIconUrl(name) {
  return `https://ddragon.leagueoflegends.com/cdn/${ddVersion}/img/champion/${champKey(name)}.png`;
}

// The 380px tile: about 0.6 MB decoded, against 3.3 MB for the full splash.
function champArtUrl(name) {
  return `https://ddragon.leagueoflegends.com/cdn/img/champion/tiles/${champKey(name)}_0.jpg`;
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

function wrClass(pct) {
  return pct >= 50 ? 'good' : 'bad';
}

// ─── Rank history ─────────────────────────────────────────────────────────────
const DAY_SECONDS = 86400;

/** A rank as one number, so readings can be compared: 100 per division, 400 per tier. */
function ladderValue(point) {
  const tier = TIER_ORDER[(point.tier || '').toUpperCase()] ?? -1;
  if (tier < 0) return null;
  const lp = Number(point.lp || 0);
  // Master and above share one LP ladder with no divisions.
  if (tier >= TIER_ORDER.MASTER) return TIER_ORDER.MASTER * 400 + lp;
  const division = DIVISION_ORDER[(point.division || '').toUpperCase()] ?? 1;
  return tier * 400 + (division - 1) * 100 + lp;
}

/**
 * The LP change over the last week, or since the first reading when there is
 * less than a week of them, and the readings of the last 30 days for a small line.
 */
function rankTrend(history, now = Date.now() / 1000) {
  const points = (history || [])
    .map(point => ({ at: point.at, value: ladderValue(point) }))
    .filter(point => point.value != null);
  if (points.length < 2) return null;

  const last = points[points.length - 1];
  const weekAgo = points.filter(point => point.at <= now - 7 * DAY_SECONDS).pop();
  const from = weekAgo || points[0];
  const period = weekAgo
    ? '7d'
    : `since ${new Date(from.at * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' })}`;

  const start = now - 30 * DAY_SECONDS;
  const before = points.filter(point => point.at < start).pop();
  const series = [
    ...(before ? [{ at: start, value: before.value }] : []),
    ...points.filter(point => point.at >= start),
    { at: now, value: last.value },
  ];
  return { delta: last.value - from.value, period, series };
}

function sparklineHTML(series) {
  const width = 44, height = 14;
  const values = series.map(point => point.value);
  const low = Math.min(...values), high = Math.max(...values);
  const first = series[0].at, span = series[series.length - 1].at - first;
  const points = series.map(point => {
    const x = span > 0 ? (point.at - first) / span * width : width;
    const y = high > low ? height - 1 - (point.value - low) / (high - low) * (height - 2) : height / 2;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(' ');
  return `<svg class="spark" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" aria-hidden="true"><polyline points="${points}" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round"/></svg>`;
}

function trendHTML(history) {
  const trend = rankTrend(history);
  if (!trend) return '';
  const { delta, period, series } = trend;
  const text = delta === 0 ? `No LP change · ${period}` : `${delta > 0 ? '▲' : '▼'} ${Math.abs(delta)} LP · ${period}`;
  const tone = delta > 0 ? 'good' : delta < 0 ? 'bad' : '';
  return `<span class="trend ${tone}" title="Solo/Duo rank over the last 30 days">${sparklineHTML(series)}<span>${esc(text)}</span></span>`;
}

// ─── Last played and in game ──────────────────────────────────────────────────
function agoText(ms) {
  const minutes = Math.max(0, Math.round((Date.now() - ms) / 60000));
  if (minutes < 2) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days === 1) return 'yesterday';
  if (days < 60) return `${days} days ago`;
  return `${Math.round(days / 30)} months ago`;
}

function clockText(when) {
  return new Date(when).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** A line for a game in progress. It is true as of the last refresh, and says so. */
function liveHTML(stats) {
  const live = stats?.live;
  if (!live) return '';
  const what = [live.queue === 'In a game' ? null : live.queue, live.champion].filter(Boolean).join(' · ');
  const started = live.startedAt > 0 ? `started ${clockText(live.startedAt)}` : 'loading in';
  const seen = stats.fetchedAt ? ` at ${clockText(stats.fetchedAt)}` : '';
  return `<div class="live" title="Seen in a game at the last refresh${esc(seen)}">
    <span class="live-dot"></span><strong>In game</strong>${what ? `<span>${esc(what)}</span>` : ''}<span class="live-when">${esc(started)}</span>
  </div>`;
}

function tagsHTML(account) {
  const tags = account.tags || [];
  if (tags.length === 0) return '';
  return `<span class="tags">${tags.map(tag =>
    `<button class="tag js-tag" data-tag="${esc(tag)}" title="Show accounts tagged ${esc(tag)}">${esc(tag)}</button>`).join('')}</span>`;
}

function queueHTML(title, rank, extra = '') {
  const ranked = isRanked(rank);
  const total = ranked ? rank.wins + rank.losses : 0;
  const pct = total > 0 ? Math.round(rank.wins / total * 100) : null;
  return `<div class="queue">
    <span class="stat-label">${title}</span>
    <span class="stat-value" style="color:${ranked ? tierColor(rank) : 'var(--dim)'}">${esc(tierName(rank))}</span>
    ${ranked ? `<span class="queue-line"><span>${esc(rank.lp)} LP · ${esc(rank.wins)}W ${esc(rank.losses)}L</span>${pct != null ? `<span class="${wrClass(pct)}">${pct}%</span>` : ''}</span>
      <span class="wr-track"><span class="wr-fill" style="width:${pct ?? 0}%"></span></span>${extra}` : ''}
  </div>`;
}

function recentHTML(recent, lastRankedAt) {
  if (!recent) return '';
  const lastPlayed = lastRankedAt ? `<span class="ago" title="Last ranked Solo/Duo game: ${esc(new Date(lastRankedAt).toLocaleString())}"> · ${esc(agoText(lastRankedAt))}</span>` : '';
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

  return `<div class="recent">
    <div class="recent-head">
      <span class="stat-label">Last ${esc(recent.games)} ranked${lastPlayed}</span>
      <span class="recent-record">${streakText ? `<span class="streak ${latestWin ? 'good' : 'bad'}" title="Current ${latestWin ? 'win' : 'loss'} streak">${streakText}</span> ` : ''}${esc(recent.wins)}W ${esc(recent.losses)}L <span class="${wrClass(pct)}">${pct}%</span></span>
    </div>
    <div class="games" aria-label="Last ${esc(recent.games)} ranked games, oldest to newest: ${esc(recent.wins)} wins, ${esc(recent.losses)} losses">${games}</div>
  </div>`;
}

// Which queue's champions a card shows. Saved per account, Solo/Duo by default.
function champQueueFor(accountId) {
  return localStorage.getItem(`champQueue:${accountId}`) === 'flex' ? 'flex' : 'solo';
}

function champListFor(stats, queue) {
  return ((queue === 'flex' ? stats?.flexChampions : stats?.topChampions) || []).slice(0, 5);
}

function champPeriod(stats, queue) {
  if (queue === 'flex') return 'Flex';
  return stats?.championScope === 'season' ? 'Solo/Duo' : `last ${esc(stats?.recent?.games ?? stats?.topChampions?.length)}`;
}

function highlightsHTML(stats, queue) {
  const top = champListFor(stats, queue)[0];
  const recent = stats?.recent;
  const role = recent?.mainRole && ROLE_NAMES[recent.mainRole];
  if (!top && !role) return '';
  return `<div class="highlights">
    ${top ? `<div class="highlight">
      <span class="stat-label">Most played, ${champPeriod(stats, queue)}</span>
      <span class="highlight-value">${esc(top.name)} <span class="highlight-sub">${esc(top.games)} G</span></span>
    </div>` : ''}
    ${role ? `<div class="highlight" title="Riot has no all-time role stat, so this comes from recent ranked games.">
      <span class="stat-label">Main role, last ${esc(recent.games)}</span>
      <span class="highlight-value">${esc(role)} <span class="highlight-sub">${esc(recent.mainRoleGames)} of ${esc(recent.games)}</span></span>
    </div>` : ''}
  </div>`;
}

function champSectionHTML(account, queue) {
  const stats = account.stats;
  if (!stats) return '';
  const champs = champListFor(stats, queue);
  const id = esc(account.id);
  const title = queue === 'solo' && stats.championScope !== 'season'
    ? `Most played, last ${esc(stats.recent?.games ?? champs.length)} ranked`
    : 'Most played this season';
  const empty = queue === 'flex'
    ? (stats.flexChampions ? 'No Flex games this season.' : 'Refresh to load Flex champions.')
    : 'No Solo/Duo games yet.';
  return `<div class="card-champs">
    <div class="champs-head">
      <span class="stat-label">${title}</span>
      <div class="queue-switch">
        <button class="js-champ-queue ${queue === 'solo' ? 'active' : ''}" data-account="${id}" data-queue="solo">Solo/Duo</button>
        <button class="js-champ-queue ${queue === 'flex' ? 'active' : ''}" data-account="${id}" data-queue="flex">Flex</button>
      </div>
    </div>
    ${champs.length ? champs.map(champRowHTML).join('') : `<span class="hint">${empty}</span>`}
  </div>`;
}

function champRowHTML(c) {
  const wr = c.winRate != null ? Math.round(c.winRate) : null;
  return `<div class="champ" title="${esc(c.name)}">
    <span class="champ-icon">${esc(c.name.slice(0, 2))}<img src="${esc(champIconUrl(c.name))}" alt=""></span>
    <span class="champ-name">${esc(c.name)}</span>
    <span class="champ-cell" title="${esc(c.games)} games">${esc(c.games)} G</span>
    <span class="champ-cell ${wr != null ? wrClass(wr) : ''}">${wr != null ? `${wr}%` : '-'}</span>
  </div>`;
}

function buildCard(account) {
  const stats = account.stats;
  const solo = stats?.solo;
  const id = esc(account.id);
  const queue = champQueueFor(account.id);
  // The art follows the switch, but keeps a champion when that queue has none.
  const artChampion = champListFor(stats, queue)[0]?.name ?? champListFor(stats, 'solo')[0]?.name;
  const updated = stats?.fetchedAt
    ? `Updated ${new Date(stats.fetchedAt).toLocaleString()}`
    : 'No stats yet';

  const card = document.createElement('article');
  card.className = 'card';
  card.dataset.row = account.id;
  card.innerHTML = `
    ${artChampion ? `<img class="card-art" src="${esc(champArtUrl(artChampion))}" alt="">` : ''}
    <div class="card-head">
      ${avatarHTML(account, 'medium')}
      <div class="card-id">
        <span class="card-name" title="${esc(account.label)}">${esc(account.label)}</span>
        <span class="sub-line">
          <button class="link js-opgg" data-url="${esc(buildOpggUrl(account.riotId, account.region))}" title="Open on op.gg">${esc(account.riotId)}</button>
          <span class="region">${esc(account.region)}</span>
        </span>
        <span class="sub-line meta-line">
          ${stats?.summonerLevel != null ? `<span class="stat-sub">Level ${esc(stats.summonerLevel)}</span>` : ''}
          ${profileLinksHTML(account)}
        </span>
        ${tagsHTML(account)}
      </div>
      <button class="play card-play js-launch" data-id="${id}" aria-label="Play ${esc(account.label)}" title="${playTitle(id)}">${ICONS.play}</button>
    </div>

    ${liveHTML(stats)}

    ${highlightsHTML(stats, queue)}

    <div class="queues" title="${esc(updated)}">
      ${queueHTML('Solo / Duo', solo, trendHTML(account.rankHistory))}
      ${queueHTML('Flex', stats?.flex)}
    </div>

    ${recentHTML(stats?.recent, stats?.lastRankedAt)}

    ${champSectionHTML(account, queue)}

    ${!stats?.recent ? `<span class="hint">${stats ? 'Refresh for recent-game analytics.' : 'No stats yet. Refresh to fetch them.'}</span>` : ''}

    <div class="card-foot">
      <button class="card-action js-copy-user" data-id="${id}" title="Copy username">${ICONS.user}<span>Username</span></button>
      <button class="card-action js-copy-pw" data-id="${id}" title="Copy password">${ICONS.key}<span>Password</span></button>
      <button class="icon-btn js-reveal" data-id="${id}" aria-label="Show username and password" title="Show username and password">${ICONS.eye}</button>
      <span class="toolbar-spacer"></span>
      <button class="icon-btn js-pin${account.pinned ? ' on' : ''}" data-id="${id}" aria-pressed="${account.pinned ? 'true' : 'false'}" aria-label="${account.pinned ? 'Unpin' : 'Pin to the top'}" title="${account.pinned ? 'Unpin' : 'Pin to the top'}">${ICONS.pin}</button>
      <button class="icon-btn js-refresh" data-id="${id}" aria-label="Refresh stats" title="Refresh stats">${ICONS.refresh}</button>
      <button class="icon-btn js-edit" data-id="${id}" aria-label="Edit or delete" title="Edit or delete">${ICONS.edit}</button>
    </div>`;
  return card;
}

// ─── Render ───────────────────────────────────────────────────────────────────
const buildAccount = LOW ? buildRow : buildCard;

function renderList() {
  const shown = sortedAccounts.filter(matchesSearch);
  accountList.replaceChildren(...shown.map(buildAccount));
  emptyState.classList.toggle('hidden', allAccounts.length > 0);
  noMatch.classList.toggle('hidden', allAccounts.length === 0 || shown.length > 0);
  // A search hides cards only for a moment, so the window keeps its size.
  if (!searchQuery) fitWindow();
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
  // Pinned accounts lead, in the same order among themselves. The sort is stable.
  sortedAccounts.sort((a, b) => Number(!!b.pinned) - Number(!!a.pinned));
}

/** Every word typed must appear in the label, Riot ID, region or a tag. */
function matchesSearch(account) {
  if (!searchQuery) return true;
  const text = [account.label, account.riotId, account.region, ...(account.tags || [])].join('\n').toLowerCase();
  return searchQuery.split(/\s+/).every(word => text.includes(word));
}

function setSearch(text) {
  searchBox.value = text;
  searchQuery = text.trim().toLowerCase();
  renderList();
}
searchBox.addEventListener('input', () => setSearch(searchBox.value));
searchBox.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && searchBox.value) { e.stopPropagation(); setSearch(''); }
});

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
  const queueBtn = e.target.closest('.js-champ-queue');
  if (queueBtn) {
    const { account: accountId, queue } = queueBtn.dataset;
    localStorage.setItem(`champQueue:${accountId}`, queue);
    const acc = allAccounts.find(a => a.id === accountId);
    if (acc) accountList.querySelector(`[data-row="${accountId}"]`)?.replaceWith(buildAccount(acc));
    return;
  }

  const tagBtn = e.target.closest('.js-tag');
  if (tagBtn) {
    setSearch(tagBtn.dataset.tag);
    return;
  }

  const pinBtn = e.target.closest('.js-pin');
  if (pinBtn) {
    const account = allAccounts.find(a => a.id === pinBtn.dataset.id);
    if (!account) return;
    pinBtn.disabled = true;
    try {
      await invoke('update_account', { id: account.id, updates: { pinned: !account.pinned } });
      await renderAccounts();
    } catch (err) {
      pinBtn.disabled = false;
      showToast(String(err) || 'Could not pin the account', 'error');
    }
    return;
  }

  const profileBtn = e.target.closest('.js-opgg, .js-profile');
  if (profileBtn) {
    invoke('open_external', { url: profileBtn.dataset.url }).catch(() => {});
    return;
  }

  const btn = e.target.closest('button[data-id]');
  if (!btn) return;
  const id = btn.dataset.id;

  if (btn.classList.contains('js-launch')) {
    await handleLaunchAccount(id);
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
  } else if (btn.classList.contains('js-reveal')) {
    await toggleCredentials(id);
  }
});

// Swaps only the icon, so a label that is showing a revealed value is left alone.
function showCopied(btn) {
  if (btn.classList.contains('copied')) return;
  const original = btn.querySelector('svg').outerHTML;
  btn.querySelector('svg').outerHTML = ICONS.check;
  btn.classList.add('copied');
  setTimeout(() => {
    btn.querySelector('svg').outerHTML = original;
    btn.classList.remove('copied');
  }, 1000);
}

// ─── Show username and password ───────────────────────────────────────────────
// Shown in place of the two labels, and hidden again after a short while.
const REVEAL_MS = 15_000;
const revealTimers = {};

function setCredentialLabels(card, username, password) {
  const shown = username != null;
  const user = card.querySelector('.js-copy-user');
  const pass = card.querySelector('.js-copy-pw');
  user.querySelector('span').textContent = shown ? username : 'Username';
  pass.querySelector('span').textContent = shown ? password : 'Password';
  user.title = shown ? `Copy username: ${username}` : 'Copy username';
  pass.title = 'Copy password';
  card.classList.toggle('revealed', shown);
  const eye = card.querySelector('.js-reveal');
  eye.innerHTML = shown ? ICONS.eyeOff : ICONS.eye;
  eye.title = shown ? 'Hide username and password' : 'Show username and password';
  eye.setAttribute('aria-label', eye.title);
}

function hideCredentials(id) {
  clearTimeout(revealTimers[id]);
  delete revealTimers[id];
  const card = accountList.querySelector(`[data-row="${id}"]`);
  if (card?.classList.contains('revealed')) setCredentialLabels(card, null, null);
}

async function toggleCredentials(id) {
  if (revealTimers[id]) return hideCredentials(id);
  const acc = allAccounts.find(a => a.id === id);
  const card = accountList.querySelector(`[data-row="${id}"]`);
  if (!acc || !card) return;
  try {
    const password = await invoke('get_password', { id });
    setCredentialLabels(card, acc.username, password);
    revealTimers[id] = setTimeout(() => hideCredentials(id), REVEAL_MS);
  } catch (err) {
    showToast(String(err), 'error');
  }
}

/** What to queue after sign-in for an account: { queue, first, second }, or null. */
function queueSettingFor(accountId) {
  try {
    const saved = JSON.parse(localStorage.getItem(`autoQueue:${accountId}`) || 'null');
    return saved && (saved.queue === 'solo' || saved.queue === 'flex') ? saved : null;
  } catch {
    return null;
  }
}

/** The Play button's tooltip says what it will do for this account. */
function playTitle(accountId) {
  const auto = queueSettingFor(accountId);
  if (!auto) return 'Sign in and open League';
  return `Sign in and queue ${auto.queue === 'flex' ? 'Ranked Flex' : 'Ranked Solo/Duo'}`;
}

function saveQueueSetting(accountId, setting) {
  if (setting) localStorage.setItem(`autoQueue:${accountId}`, JSON.stringify(setting));
  else localStorage.removeItem(`autoQueue:${accountId}`);
}

async function handleLaunchAccount(id) {
  const acc = allAccounts.find(a => a.id === id);
  if (!acc) return;
  // Signing in types into the Riot Client, so only one can run at a time.
  const playButtons = () => appRoot.querySelectorAll('.js-launch');
  playButtons().forEach(b => { b.disabled = true; });
  setProgressToast(`Signing in to ${acc.label}...`, 'launch-toast');
  try {
    const message = await invoke('launch_account', { id });
    const auto = queueSettingFor(id);
    if (!auto) {
      showToast(message, 'success', 6000);
    } else {
      const queueName = auto.queue === 'flex' ? 'Ranked Flex' : 'Ranked Solo/Duo';
      setProgressToast(`${message} Queueing ${queueName} once League is ready...`, 'launch-toast');
      const queued = await invoke('queue_up', { id, queue: auto.queue, firstRole: auto.first, secondRole: auto.second });
      showToast(queued, 'success', 8000);
    }
  } catch (err) {
    showToast(String(err) || 'Sign-in failed', 'error', 9000);
  } finally {
    setProgressToast(null, 'launch-toast');
    playButtons().forEach(b => { b.disabled = false; });
  }
}

/** Refreshes one account and redraws it in place. Never throws. */
async function refreshAccount(id) {
  const row = accountList.querySelector(`[data-row="${id}"]`);
  const btn = row?.querySelector('.js-refresh');
  if (btn) { btn.disabled = true; btn.classList.add('busy'); }

  try {
    const result = await invoke('refresh_stats', { id });
    const acc = allAccounts.find(a => a.id === id);
    if (acc) {
      acc.stats = result.stats;
      acc.rankHistory = result.rankHistory;
      accountList.querySelector(`[data-row="${id}"]`)?.replaceWith(buildAccount(acc));
      if (!refreshingAll && !searchQuery) fitWindow();
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
    saveQueueSetting(acc.id, null);
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
  scoutRun++;
}

/** Each look at a lobby gets a number; a newer one makes the older stop quietly. */
let scoutRun = 0;

function lobbyName(player) {
  return (player.gameName && player.tagLine) ? `${player.gameName}#${player.tagLine}` : (player.summonerName || 'Hidden');
}

function sameRiotId(a, b) {
  const plain = id => String(id || '').replace(/\s+/g, '').toLowerCase();
  return !!a && !!b && plain(a) === plain(b);
}

function renderLobbyPlayers(players, me) {
  if (!players || players.length === 0) {
    lobbyList.innerHTML = '<div class="hint">No players yet. Open this during champ select.</div>';
    return;
  }
  lobbyList.innerHTML = players.map((player, index) => {
    const name = lobbyName(player);
    return `<div class="lobby-player" data-player="${index}">
      <div class="lobby-line">
        <span class="lobby-name">${esc(name)}${sameRiotId(name, me) ? '<span class="you">you</span>' : ''}</span>
        <span class="lobby-rank"></span>
      </div>
      <div class="lobby-detail hint"></div>
    </div>`;
  }).join('');
}

/** A teammate's rank, and one line on their season, latest games and usual roles. */
function scoutHTML(report) {
  const solo = report.solo;
  const ranked = isRanked(solo);
  const played = ranked ? solo.wins + solo.losses : 0;
  const rate = played > 0 ? Math.round(solo.wins / played * 100) : null;
  const run = Math.abs(report.streak) >= 2 ? ` (${report.streak > 0 ? 'W' : 'L'}${Math.abs(report.streak)})` : '';
  const roles = (report.roles || []).slice(0, 2).map(role => ROLE_NAMES[role.role] || capitalise(role.role)).join(', ');

  const parts = [];
  if (rate != null) parts.push(`<span class="${wrClass(rate)}">${rate}%</span> over ${esc(played)} games`);
  if (report.recentGames > 0) parts.push(`last ${esc(report.recentGames)}: ${esc(report.recentWins)}W ${esc(report.recentGames - report.recentWins)}L${run}`);
  if (roles) parts.push(`plays ${esc(roles)}`);
  return {
    rank: `${esc(tierName(solo))}${ranked ? ` · ${esc(solo.lp)} LP` : ''}`,
    color: ranked ? tierColor(solo) : 'var(--dim)',
    detail: parts.join(' · ') || 'No ranked Solo/Duo games found',
  };
}

function teamReadHTML(team, looked, teammates) {
  const tone = team.estimate >= 53 ? 'good' : team.estimate <= 47 ? 'bad' : '';
  const notes = team.notes.map(note =>
    `<li class="${note.points >= 0.5 ? 'good' : note.points <= -0.5 ? 'bad' : ''}">${esc(note.text)}</li>`).join('');
  return `<div class="team-estimate ${tone}"><strong>${esc(team.estimate)}%</strong><span>rough win estimate</span></div>
    <ul class="team-notes">${notes || '<li>Nothing stands out.</li>'}</ul>
    <p class="hint">From the rank and latest ranked games of ${esc(looked)} of ${esc(teammates)} teammates. The enemy team is unknown, so treat this as a hint.</p>`;
}

/** In champion select: looks up each teammate in turn, then reads the team as a whole. */
async function scoutLobby(data) {
  const run = ++scoutRun;
  const read = document.getElementById('lobby-read');
  read.classList.add('hidden');
  read.innerHTML = '';
  if (!data.inChampSelect) return;

  const teammates = data.players
    .map((player, index) => ({ index, riotId: lobbyName(player), named: !!(player.gameName && player.tagLine) }))
    .filter(mate => mate.named && !sameRiotId(mate.riotId, data.me));
  if (teammates.length === 0) return;

  const reports = [];
  for (const mate of teammates) {
    const row = lobbyList.querySelector(`[data-player="${mate.index}"]`);
    const detail = row?.querySelector('.lobby-detail');
    if (detail) detail.innerHTML = `${LOADER}Looking up...`;
    try {
      const report = await invoke('scout_player', { riotId: mate.riotId, region: data.region || 'NA' });
      if (run !== scoutRun) return;
      reports.push(report);
      const shown = scoutHTML(report);
      const rank = row?.querySelector('.lobby-rank');
      if (rank) { rank.textContent = ''; rank.insertAdjacentHTML('beforeend', shown.rank); rank.style.color = shown.color; }
      if (detail) detail.innerHTML = shown.detail;
    } catch (err) {
      if (run !== scoutRun) return;
      if (detail) detail.textContent = String(err) || 'No data';
    }
  }
  if (reports.length === 0) return;

  try {
    const team = await invoke('read_team', { teammates: reports, positions: data.positions || [] });
    if (run !== scoutRun) return;
    read.innerHTML = teamReadHTML(team, reports.length, teammates.length);
    read.classList.remove('hidden');
  } catch (err) {
    console.warn('Could not read the team:', err);
  }
}

/** Shows a lobby and, in champion select, starts looking up the teammates. */
function showLobby(data) {
  activeLobbyData = data;
  lobbyMeta.textContent = `${data.phase} · ${data.region || 'NA'} · ${data.players.length} players`;
  renderLobbyPlayers(data.players, data.me);
  scoutLobby(data);
}

async function fetchLobby() {
  const refreshBtn = document.getElementById('lobby-refresh-btn');
  refreshBtn.disabled = true;
  scoutRun++;
  lobbyError.classList.add('hidden');
  document.getElementById('lobby-read').classList.add('hidden');
  lobbyMeta.innerHTML = `${LOADER}Checking League client...`;
  lobbyList.innerHTML = '';

  try {
    showLobby(await invoke('get_lobby_view'));
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
    prompt.className = 'toast sticky prompt';
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

const queueField = document.getElementById('field-queue');
const role1Field = document.getElementById('field-role1');
const role2Field = document.getElementById('field-role2');
queueField.addEventListener('change', syncQueueFields);
role1Field.addEventListener('change', syncQueueFields);

/** Roles only show when queueing; Fill takes no second role. */
function syncQueueFields() {
  const queueing = queueField.value !== '';
  accountModal.querySelectorAll('.queue-role').forEach(el => el.classList.toggle('hidden', !queueing));
  role2Field.disabled = role1Field.value === 'FILL';
  if (role2Field.disabled) role2Field.value = '';
}

function setQueueFields(setting) {
  queueField.value = setting?.queue || '';
  role1Field.value = setting?.first || '';
  role2Field.value = setting?.second || '';
  syncQueueFields();
}

/** The first "Account#N" no account uses yet. */
function nextAccountLabel() {
  const used = new Set(allAccounts.map(a => a.label.trim().toLowerCase()));
  let n = 1;
  while (used.has(`account#${n}`)) n++;
  return `Account#${n}`;
}

function openAddModal() {
  modalTitle.textContent = 'Add account';
  editIdInput.value = '';
  clearModalFields();
  modalDeleteBtn.classList.add('hidden');
  document.getElementById('field-password').placeholder = '';
  document.getElementById('field-label').placeholder = nextAccountLabel();
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
  document.getElementById('field-label').placeholder = acc.label;
  document.getElementById('field-username').value = acc.username;
  document.getElementById('field-riotid').value   = acc.riotId;
  document.getElementById('field-region').value   = acc.region;
  setQueueFields(queueSettingFor(id));
  document.getElementById('field-tags').value = (acc.tags || []).join(', ');
  document.getElementById('field-pinned').checked = !!acc.pinned;
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
  setQueueFields(null);
  document.getElementById('field-tags').value = '';
  document.getElementById('field-pinned').checked = false;
  document.getElementById('modal-error').classList.add('hidden');
}

function showModalError(msg) {
  const el = document.getElementById('modal-error');
  el.textContent = msg;
  el.classList.remove('hidden');
}

async function handleModalSave() {
  const id       = editIdInput.value;
  // A blank label is filled in: the next "Account#N" when adding, the current one when editing.
  const label    = document.getElementById('field-label').value.trim()
    || allAccounts.find(a => a.id === id)?.label
    || nextAccountLabel();
  const username = document.getElementById('field-username').value.trim();
  const password = document.getElementById('field-password').value;
  const riotId   = document.getElementById('field-riotid').value.trim();
  const region   = document.getElementById('field-region').value;
  const tags     = document.getElementById('field-tags').value.split(',').map(tag => tag.trim()).filter(Boolean);
  const pinned   = document.getElementById('field-pinned').checked;

  if (!username) return showModalError('Username is required.');
  if (!riotId)   return showModalError('Riot ID is required.');
  if (!/^.+#.+$/.test(riotId)) return showModalError('Riot ID must look like GameName#TAG.');
  if (!id && !password) return showModalError('Password is required.');

  const first = role1Field.value;
  const second = first === 'FILL' ? '' : role2Field.value;
  const queueSetting = queueField.value ? { queue: queueField.value, first, second } : null;
  if (queueSetting && !first) return showModalError('Pick a first role to queue.');
  if (queueSetting && first !== 'FILL' && !second) return showModalError('Pick a second role to queue.');
  if (queueSetting && first === second) return showModalError('The two roles must be different.');

  const saveBtn = document.getElementById('modal-save-btn');
  saveBtn.disabled = true;

  try {
    if (id) {
      const updates = { label, username, riotId, region, tags, pinned };
      if (password) updates.password = password;
      await invoke('update_account', { id, updates });
      saveQueueSetting(id, queueSetting);
      closeModal();
      await renderAccounts();
    } else {
      const newId = await invoke('add_account', { account: { label, username, password, riotId, region, tags, pinned } });
      saveQueueSetting(newId, queueSetting);
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
// Theme
const themeButtons = [...document.querySelectorAll('#theme-picker button')];
function showTheme() {
  themeButtons.forEach(button => button.classList.toggle('active', button.dataset.theme === currentTheme));
}
themeButtons.forEach(button => button.addEventListener('click', () => {
  currentTheme = applyTheme(button.dataset.theme);
  try { localStorage.setItem('theme', currentTheme); } catch {}
  showTheme();
}));

// Backup
const backupStatus = document.getElementById('backup-status');
const backupPinModal = document.getElementById('backup-pin-modal');
const backupPinInput = document.getElementById('backup-pin-input');
const backupPinError = document.getElementById('backup-pin-error');
let backupPath = null;

document.getElementById('backup-export-btn').addEventListener('click', async (e) => {
  // Kept here: an event forgets its button once the handler waits.
  const button = e.currentTarget;
  button.disabled = true;
  try {
    const saved = await invoke('export_backup');
    if (saved) {
      backupStatus.textContent = `Saved to ${saved}`;
      showToast('Backup saved.', 'success');
    }
  } catch (err) {
    showToast(String(err) || 'Could not save the backup', 'error', 8000);
  } finally {
    button.disabled = false;
  }
});

document.getElementById('backup-import-btn').addEventListener('click', async () => {
  try {
    const chosen = await invoke('choose_backup');
    if (!chosen) return;
    backupPath = chosen;
    document.getElementById('backup-pin-file').textContent = chosen.split(/[\\/]/).pop();
    backupPinInput.value = '';
    backupPinError.classList.add('hidden');
    backupPinModal.classList.remove('hidden');
    backupPinInput.focus();
  } catch (err) {
    showToast(String(err) || 'Could not open the backup', 'error', 8000);
  }
});

function closeBackupPin() {
  backupPinModal.classList.add('hidden');
  backupPinInput.value = '';
}
document.getElementById('backup-pin-cancel').addEventListener('click', closeBackupPin);
backupPinInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') document.getElementById('backup-pin-ok').click();
});
document.getElementById('backup-pin-ok').addEventListener('click', async (e) => {
  const password = backupPinInput.value;
  if (!password) {
    backupPinError.textContent = 'Enter the PIN of the backup.';
    backupPinError.classList.remove('hidden');
    return;
  }
  const button = e.currentTarget;
  button.disabled = true;
  try {
    const { added, skipped } = await invoke('import_backup', { path: backupPath, password });
    closeBackupPin();
    await renderAccounts();
    const already = skipped === 1 ? '1 was already here.' : `${skipped} were already here.`;
    const message = added === 0
      ? `Nothing to add: ${skipped === 1 ? 'the account is' : `all ${skipped} accounts are`} already here.`
      : `Added ${added} account${added === 1 ? '' : 's'}.${skipped ? ` ${already}` : ''}`;
    backupStatus.textContent = message;
    showToast(message, added ? 'success' : undefined, 6000);
  } catch (err) {
    backupPinError.textContent = String(err) || 'Could not import the backup.';
    backupPinError.classList.remove('hidden');
  } finally {
    button.disabled = false;
  }
});

document.getElementById('settings-btn').addEventListener('click', () => {
  showTheme();
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
  if (!silent) updateStatusEl.innerHTML = `${LOADER}Checking for updates...`;
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
  updateStatusEl.innerHTML = `${LOADER}Downloading update...`;
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
/** A toast that stays until replaced; null removes it. Each id is its own toast. */
function setProgressToast(message, id = 'progress-toast') {
  let toast = document.getElementById(id);
  if (message == null) { toast?.remove(); return; }
  if (!toast) {
    toast = document.createElement('div');
    toast.id = id;
    toast.className = 'toast sticky';
    toastContainer.appendChild(toast);
  }
  toast.innerHTML = `${LOADER}${esc(message)}`;
}

function showToast(message, type = 'info', duration = type === 'error' ? 6000 : 3000) {
  const toasts = toastContainer.querySelectorAll('.toast:not(.sticky)');
  if (toasts.length >= 3) toasts[0].remove();

  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.textContent = message;
  toastContainer.appendChild(toast);
  setTimeout(() => toast.remove(), duration);
}

// ─── Window size ──────────────────────────────────────────────────────────────
// The window opens just big enough for the accounts: whole cards only, as many as
// the screen has room for, so no card is cut in half and no column is left empty.
// It follows the accounts until the user resizes or maximizes it, then leaves it alone.
const FIT = LOW
  ? { itemWidth: 744, gap: 0, minWidth: 760, minSize: [760, 300], startCols: 1, maxCols: 1, emptyHeight: 440 }
  : { itemWidth: 360, gap: 12, minWidth: 660, minSize: [660, 420], startCols: 3, maxCols: 4, emptyHeight: 520 };
const SCREEN_MARGIN = 40;

/** The size last set here; null once the user has taken over. */
let autoSize = null;
let fitting = Promise.resolve();

/** Works out the window size for the accounts on screen, without resizing anything. */
function planWindow() {
  const scroller = accountList.parentElement;
  const items = [...accountList.children];
  const style = getComputedStyle(scroller);
  const sidePad = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
  const bottomPad = parseFloat(style.paddingBottom);
  const maxWidth = screen.availWidth - SCREEN_MARGIN;
  const maxHeight = screen.availHeight - SCREEN_MARGIN;
  const widthFor = cols => Math.max(FIT.minWidth, sidePad + cols * FIT.itemWidth + (cols - 1) * FIT.gap);

  if (items.length === 0) return { width: widthFor(Math.min(2, FIT.maxCols)), height: FIT.emptyHeight };

  const colsOnScreen = Math.floor((maxWidth - sidePad + FIT.gap) / (FIT.itemWidth + FIT.gap));
  const maxCols = Math.max(1, Math.min(FIT.maxCols, colsOnScreen, items.length));
  let plan;
  // Start at the usual width; go wider only when that shows more accounts.
  for (let cols = Math.min(FIT.startCols, maxCols); cols <= maxCols; cols++) {
    const width = widthFor(cols);
    // Lay the list out as it would be at that width, measure, and put it back.
    accountList.style.width = `${width - sidePad}px`;
    const list = accountList.getBoundingClientRect();
    const rows = new Map();
    for (const item of items) {
      const box = item.getBoundingClientRect();
      const row = Math.round(box.top);
      rows.set(row, Math.max(rows.get(row) || 0, box.bottom));
    }
    accountList.style.width = '';

    const above = list.top + scroller.scrollTop;
    const heights = [...rows.values()].sort((a, b) => a - b)
      .map(bottom => Math.ceil(above + (bottom - list.top) + bottomPad));
    const shown = Math.max(1, heights.filter(height => height <= maxHeight).length);
    plan = {
      width,
      height: Math.max(FIT.minSize[1], Math.min(heights[shown - 1], maxHeight)),
    };
    if (shown === heights.length) break;
  }
  return plan;
}

/**
 * Sizes the window to the accounts. `opening` is the first fit after unlock; later
 * calls only act while the window still has the size set here.
 */
function fitWindow({ opening = false } = {}) {
  fitting = fitting.then(async () => {
    try {
      if (!opening) {
        if (!autoSize) return;
        const moved = Math.abs(innerWidth - autoSize.width) > 2 || Math.abs(innerHeight - autoSize.height) > 2;
        if (moved || await appWindow.isMaximized()) { autoSize = null; return; }
      }
      const plan = planWindow();
      if (autoSize && plan.width === autoSize.width && plan.height === autoSize.height) return;

      const { LogicalSize } = window.__TAURI__.dpi;
      await appWindow.setMinSize(new LogicalSize(FIT.minSize[0], FIT.minSize[1]));
      // Wait for the page to see the new size, so the next call doesn't mistake it for
      // the user's. A resize that didn't take is asked for again, so the window can't
      // stay at the size of the PIN screen.
      const applied = () => Math.abs(innerWidth - plan.width) <= 2 && Math.abs(innerHeight - plan.height) <= 2;
      for (let attempt = 0; attempt < 3 && !applied(); attempt++) {
        await appWindow.setSize(new LogicalSize(plan.width, plan.height));
        for (let i = 0; i < 40 && !applied(); i++) {
          await new Promise(done => setTimeout(done, 25));
        }
      }
      const offScreen = screenX + plan.width > screen.availLeft + screen.availWidth
        || screenY + plan.height > screen.availTop + screen.availHeight;
      if (opening || offScreen) await appWindow.center();
      if (!applied()) {
        console.warn('The window did not take the size', plan.width, plan.height);
        return;
      }
      autoSize = plan;
      try { localStorage.setItem('windowSize', JSON.stringify(plan)); } catch {}
    } catch (err) {
      console.warn('Could not size the window:', err);
    }
  });
  return fitting;
}

// ─── Init ─────────────────────────────────────────────────────────────────────
(async () => {
  // Saved stats go on screen straight away, before anything touches the network.
  // The version check reads its cache first, so icons are right on the first draw.
  const versionCheck = LOW ? null : initDataDragonVersion();
  const versionFromCache = ddVersion;
  await renderAccounts();
  await fitWindow({ opening: true });
  versionCheck?.then(() => {
    if (ddVersion !== versionFromCache) renderList();
  });
  refreshAllAccounts();
  if (autoCheckUpdates) {
    runAutoUpdate();
    setInterval(runAutoUpdate, AUTO_UPDATE_INTERVAL_MS);
  }
})();
