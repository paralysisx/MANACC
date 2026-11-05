'use strict';

// A new version is offered in a popup, on the PIN screen and in the main window.
// Nothing is downloaded or installed until the user presses Update now.
(function () {
  const LATER_KEY = 'updateLater';
  let overlay = null;
  let current = null;
  let busy = false;
  let installed = false;

  function build() {
    overlay = document.createElement('div');
    overlay.className = 'overlay hidden';
    overlay.id = 'update-modal';
    overlay.innerHTML = `
      <div class="modal update-modal" role="dialog" aria-labelledby="update-title">
        <h3 id="update-title">A new version is available</h3>
        <p id="update-versions"></p>
        <p class="hint" id="update-note">Updating takes a few seconds and restarts VaultX. Your accounts stay as they are.</p>
        <p class="error hidden" id="update-error"></p>
        <div class="modal-actions">
          <button class="btn" id="update-later-btn">Later</button>
          <button class="btn primary" id="update-now-btn">Update now</button>
        </div>
      </div>`;
    document.body.appendChild(overlay);
    overlay.querySelector('#update-later-btn').addEventListener('click', later);
    overlay.querySelector('#update-now-btn').addEventListener('click', install);
  }

  function later() {
    if (busy || !current) return;
    // Asked again the next time VaultX starts, not again in this one.
    if (!installed) { try { sessionStorage.setItem(LATER_KEY, current.version); } catch {} }
    overlay.classList.add('hidden');
  }

  async function install() {
    if (busy || !current) return;
    busy = true;
    const now = overlay.querySelector('#update-now-btn');
    const laterBtn = overlay.querySelector('#update-later-btn');
    const note = overlay.querySelector('#update-note');
    const error = overlay.querySelector('#update-error');
    error.classList.add('hidden');
    now.disabled = laterBtn.disabled = true;
    note.innerHTML = '<span class="loader"></span>Downloading the update...';

    let total = 0, done = 0;
    try {
      await current.downloadAndInstall(event => {
        if (event.event === 'Started') total = event.data?.contentLength || 0;
        if (event.event === 'Progress') {
          done += event.data?.chunkLength || 0;
          if (total > 0) note.innerHTML = `<span class="loader"></span>Downloading the update... ${Math.min(100, Math.round(done / total * 100))}%`;
        }
        if (event.event === 'Finished') note.innerHTML = '<span class="loader"></span>Installing. VaultX will restart...';
      });
      // The installer normally restarts the app before this line runs.
      note.textContent = 'The update is installed. Restart VaultX to finish.';
      installed = true;
      laterBtn.textContent = 'Close';
      laterBtn.disabled = false;
    } catch (err) {
      note.textContent = 'Updating takes a few seconds and restarts VaultX. Your accounts stay as they are.';
      error.textContent = `The update couldn't be installed: ${String(err) || 'unknown error'}`;
      error.classList.remove('hidden');
      now.disabled = laterBtn.disabled = false;
    } finally {
      busy = false;
    }
  }

  /**
   * Shows the popup for this update. Skipped when the user already chose Later for this
   * version since VaultX started, unless `evenIfPostponed` (the Settings button).
   */
  function offer(update, evenIfPostponed = false) {
    if (!update || busy) return false;
    let postponed = null;
    try { postponed = sessionStorage.getItem(LATER_KEY); } catch {}
    if (!evenIfPostponed && postponed === update.version) return false;

    if (!overlay) build();
    current = update;
    installed = false;
    overlay.querySelector('#update-later-btn').textContent = 'Later';
    overlay.querySelector('#update-error').classList.add('hidden');
    overlay.querySelector('#update-now-btn').disabled = false;
    overlay.querySelector('#update-later-btn').disabled = false;
    const versions = overlay.querySelector('#update-versions');
    versions.textContent = `VaultX ${update.version} is out. You should update.`;
    window.__TAURI__.app.getVersion()
      .then(mine => { versions.textContent = `VaultX ${update.version} is out and you have ${mine}. You should update.`; })
      .catch(() => {});
    overlay.classList.remove('hidden');
    return true;
  }

  /** Looks for a newer version and offers it. Returns the update, or null. */
  async function check() {
    if (localStorage.getItem('autoCheckUpdates') === 'false') return null;
    try {
      const update = await window.__TAURI__.updater.check();
      if (update) offer(update);
      return update || null;
    } catch (err) {
      console.warn('Update check failed:', err);
      return null;
    }
  }

  window.VaultXUpdate = { offer, check };
})();
