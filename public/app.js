// ═══════════════════════════════════════════════════════════════════════════
// App Controller — SPA routing, views, HLS player, admin panels
// ═══════════════════════════════════════════════════════════════════════════

let cameras = [];
let currentCamera = null;
let hls = null;          // live HLS instance
let dvrHls = null;       // separate DVR HLS instance (never shared with live)
let dvrMode = false;
let dvrLoaded = false;   // true once the full VOD playlist is loaded
let sliderRefreshInterval = null;
let recordingDuration = 0;
let isScrubbing = false; // true while user is dragging the slider

// Multi-View State
let mvCameras = [];
let mvHlsInstances = {}; // { camId: { hls: null, dvrHls: null, video: el } }
let mvDvrMode = false;
let mvDvrLoadedCount = 0;
let mvRecordingDuration = 0;
let mvIsScrubbing = false;
let mvSliderRefreshInterval = null;
// ── Init ──────────────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', () => {
    updateAuthUI();
    loadCameras();

    // Keep slider in sync with video playback position during DVR
    const video = document.getElementById('video-player');
    video.addEventListener('timeupdate', onVideoTimeUpdate);
});

function onVideoTimeUpdate() {
    if (!dvrMode || isScrubbing || !dvrLoaded) return;
    const video = document.getElementById('video-player');
    const slider = document.getElementById('dvr-slider');
    if (!video.duration || !isFinite(video.duration)) return;

    // Map video.currentTime → slider position
    slider.value = Math.round(video.currentTime);
    slider.max = Math.round(video.duration);
    updateTimelineLabels();
}

function updateAuthUI() {
    const userInfo = document.getElementById('user-info');
    const btnLogin = document.getElementById('btn-login');
    const btnAdmin = document.getElementById('btn-admin');
    const displayName = document.getElementById('display-username');

    if (API.isLoggedIn()) {
        userInfo.classList.remove('hidden');
        btnLogin.classList.add('hidden');
        displayName.textContent = API.username();
        if (API.isAdmin()) btnAdmin.classList.remove('hidden');
        else btnAdmin.classList.add('hidden');
    } else {
        userInfo.classList.add('hidden');
        btnLogin.classList.remove('hidden');
    }
}

// ── Navigation ────────────────────────────────────────────────────────────
function showView(id) {
    document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
    document.getElementById(id).classList.add('active');
}

function showGrid() { destroyPlayer(); destroyMultiView(); currentCamera = null; showView('view-grid'); loadCameras(); }
function showPlayer(camId) { destroyMultiView(); showView('view-player'); openCamera(camId); }
function showMultiView() { destroyPlayer(); destroyMultiView(); currentCamera = null; showView('view-multiview'); loadMvState(); renderMultiViewGrid(); }
function showAdmin() { showView('view-admin'); loadAdminCameras(); loadAdminUsers(); loadSettings(); }
function showLogin() { document.getElementById('modal-login').classList.remove('hidden'); }
function closeModals() { document.querySelectorAll('.modal').forEach(m => m.classList.add('hidden')); }

// ── Toast ─────────────────────────────────────────────────────────────────
function toast(msg, type = 'success') {
    const container = document.getElementById('toast-container');
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.textContent = msg;
    container.appendChild(el);
    setTimeout(() => { el.style.opacity = '0'; setTimeout(() => el.remove(), 300); }, 3000);
}

// ═══════════════════════════════════════════════════════════════════════════
// CAMERAS GRID
// ═══════════════════════════════════════════════════════════════════════════
async function loadCameras() {
    try {
        cameras = await API.getCameras();
        renderCameraGrid();
    } catch (e) {
        console.error('Failed to load cameras:', e);
    }
}

function renderCameraGrid() {
    const grid = document.getElementById('camera-grid');
    if (!cameras.length) {
        grid.innerHTML = `
            <div class="empty-state">
                <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" opacity="0.3">
                    <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/>
                    <circle cx="12" cy="13" r="4"/>
                </svg>
                <p>No cameras configured</p>
                <p class="sub">Login as admin to add cameras</p>
            </div>`;
        return;
    }

    grid.innerHTML = cameras.map(cam => `
        <div class="camera-card" onclick="showPlayer(${cam.id})" id="cam-card-${cam.id}">
            <div class="card-preview">
                <svg class="preview-icon" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1">
                    <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"/>
                    <circle cx="12" cy="13" r="4"/>
                </svg>
                <img class="preview-thumb" src="/api/segments/${cam.id}/thumb.jpg?t=${Date.now()}" onerror="this.style.display='none'" alt="" />
                ${cam.recording ? '<span class="card-live-badge">● LIVE</span>' : ''}
            </div>
            <div class="card-info">
                <h3>${escapeHtml(cam.name)}</h3>
                <p>${cam.location ? escapeHtml(cam.location) : 'No location'}</p>
                <span class="card-source-badge ${cam.source_type}">${cam.source_type === 'hls' ? 'HLS' : 'OBS'}</span>
            </div>
        </div>
    `).join('');
}

function filterCameras() {
    const q = document.getElementById('search-cameras').value.toLowerCase();
    document.querySelectorAll('.camera-card').forEach(card => {
        const name = card.querySelector('h3').textContent.toLowerCase();
        const loc = card.querySelector('.card-info p').textContent.toLowerCase();
        card.style.display = (name.includes(q) || loc.includes(q)) ? '' : 'none';
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// HLS PLAYER
// ═══════════════════════════════════════════════════════════════════════════
async function openCamera(camId) {
    destroyPlayer();
    try {
        currentCamera = await API.getCamera(camId);
    } catch { currentCamera = cameras.find(c => c.id === camId); }

    document.getElementById('player-camera-name').textContent = currentCamera.name;
    document.getElementById('badge-live').classList.remove('hidden');
    document.getElementById('badge-dvr').classList.add('hidden');
    document.getElementById('player-overlay').classList.remove('hidden');

    // Set initial slider range from actual recording
    await refreshSliderRange(camId);

    // Load archive dates
    loadArchiveDates(camId);

    // Start live playback
    startLivePlayback(camId);

    // Periodically update slider range while in live mode
    sliderRefreshInterval = setInterval(() => {
        if (currentCamera && !dvrMode) refreshSliderRange(currentCamera.id);
    }, 10000);
}

async function refreshSliderRange(camId) {
    try {
        const range = await API.getSegmentRange(camId);
        if (range.duration_seconds > 0) {
            const maxRewind = currentCamera.rewind_hours * 3600;
            recordingDuration = Math.min(range.duration_seconds, maxRewind);
        } else {
            recordingDuration = 60;
        }
    } catch {
        recordingDuration = 60;
    }

    if (!dvrMode) {
        const slider = document.getElementById('dvr-slider');
        slider.max = recordingDuration;
        slider.value = recordingDuration;
        updateTimelineLabels();
    }
}

function startLivePlayback(camId) {
    const video = document.getElementById('video-player');
    const url = `/api/cameras/${camId}/live.m3u8`;

    if (Hls.isSupported()) {
        hls = new Hls({
            liveSyncDurationCount: 3,
            liveMaxLatencyDurationCount: 6,
            maxBufferLength: 30,
            enableWorker: true,
        });
        hls.loadSource(url);
        hls.attachMedia(video);
        hls.on(Hls.Events.MANIFEST_PARSED, () => {
            video.play().catch(() => {});
            document.getElementById('player-overlay').classList.add('hidden');
        });
        hls.on(Hls.Events.ERROR, (_, data) => {
            if (data.fatal) {
                console.error('HLS live error:', data);
                if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
                    setTimeout(() => hls && hls.startLoad(), 3000);
                }
            }
        });
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
        video.src = url;
        video.addEventListener('loadedmetadata', () => {
            video.play().catch(() => {});
            document.getElementById('player-overlay').classList.add('hidden');
        });
    }

    dvrMode = false;
    dvrLoaded = false;
}

function destroyPlayer() {
    if (hls) { hls.destroy(); hls = null; }
    if (dvrHls) { dvrHls.destroy(); dvrHls = null; }
    if (sliderRefreshInterval) { clearInterval(sliderRefreshInterval); sliderRefreshInterval = null; }

    const video = document.getElementById('video-player');
    video.pause();
    video.removeAttribute('src');
    video.load();

    dvrMode = false;
    dvrLoaded = false;
    recordingDuration = 0;
    isScrubbing = false;
}

// ── DVR Controls ──────────────────────────────────────────────────────────

function onDvrSliderInput() {
    isScrubbing = true;
    updateTimelineLabels();

    // If full DVR playlist is loaded, seek the video in real-time while dragging
    if (dvrMode && dvrLoaded && dvrHls) {
        const video = document.getElementById('video-player');
        const slider = document.getElementById('dvr-slider');
        video.currentTime = parseFloat(slider.value);
    }
}

function onDvrSliderChange() {
    isScrubbing = false;
    const slider = document.getElementById('dvr-slider');
    const maxVal = parseFloat(slider.max);
    const val = parseFloat(slider.value);

    // If in live mode and slider is near end, stay live
    if (!dvrMode && val >= maxVal - 10) return;

    if (dvrMode && dvrLoaded) {
        // Already in DVR — just seek to the final position
        const video = document.getElementById('video-player');
        video.currentTime = val;
        video.play().catch(() => {});
    } else {
        // First scrub — enter DVR mode and load full VOD playlist
        enterDvrMode(val / maxVal);
    }
}

function enterDvrMode(seekRatio) {
    if (!currentCamera) return;

    // Completely destroy live instance — never reuse between modes
    if (hls) { hls.destroy(); hls = null; }

    dvrMode = true;
    dvrLoaded = false;
    document.getElementById('badge-live').classList.add('hidden');
    document.getElementById('badge-dvr').classList.remove('hidden');
    document.getElementById('player-overlay').classList.remove('hidden');

    const video = document.getElementById('video-player');
    const url = `/api/cameras/${currentCamera.id}/full.m3u8`;

    // Fresh HLS instance configured for VOD playback
    dvrHls = new Hls({
        enableWorker: true,
        maxBufferLength: 60,
        maxMaxBufferLength: 120,
    });
    dvrHls.loadSource(url);
    dvrHls.attachMedia(video);

    dvrHls.on(Hls.Events.MANIFEST_PARSED, () => {
        dvrLoaded = true;
        document.getElementById('player-overlay').classList.add('hidden');

        // Wait for duration to populate, then seek to requested position
        const checkDuration = () => {
            if (video.duration && isFinite(video.duration)) {
                const slider = document.getElementById('dvr-slider');
                slider.max = Math.round(video.duration);
                const targetTime = seekRatio != null ? seekRatio * video.duration : video.duration;
                video.currentTime = Math.min(targetTime, video.duration - 1);
                slider.value = Math.round(video.currentTime);
                video.play().catch(() => {});
                updateTimelineLabels();
            } else {
                setTimeout(checkDuration, 50);
            }
        };
        checkDuration();
    });

    dvrHls.on(Hls.Events.ERROR, (_, data) => {
        if (data.fatal) {
            console.error('DVR error:', data);
            if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
                setTimeout(() => dvrHls && dvrHls.startLoad(), 2000);
            }
        }
    });
}

function jumpToLive() {
    if (!currentCamera) return;
    const camId = currentCamera.id;

    // Completely destroy DVR instance
    if (dvrHls) { dvrHls.destroy(); dvrHls = null; }
    dvrMode = false;
    dvrLoaded = false;

    document.getElementById('badge-live').classList.remove('hidden');
    document.getElementById('badge-dvr').classList.add('hidden');

    const slider = document.getElementById('dvr-slider');
    slider.max = recordingDuration;
    slider.value = recordingDuration;
    updateTimelineLabels();

    startLivePlayback(camId);
}

function rewindBy(seconds) {
    if (!dvrMode || !dvrLoaded) {
        // Calculate seek ratio and enter DVR
        const slider = document.getElementById('dvr-slider');
        const maxVal = parseFloat(slider.max);
        const ratio = Math.max(0, (maxVal - seconds)) / maxVal;
        enterDvrMode(ratio);
    } else {
        // Already in DVR — seek directly within the loaded video
        const video = document.getElementById('video-player');
        video.currentTime = Math.max(0, video.currentTime - seconds);
    }
}

function updateTimelineLabels() {
    const slider = document.getElementById('dvr-slider');
    const maxVal = parseFloat(slider.max) || 1;
    const val = parseFloat(slider.value) || 0;

    if (dvrMode && dvrLoaded) {
        document.getElementById('timeline-start').textContent = '0:00';
        document.getElementById('timeline-current').textContent = formatDuration(val);
        document.getElementById('timeline-end').textContent = formatDuration(maxVal);
    } else {
        const secondsAgo = maxVal - val;
        document.getElementById('timeline-start').textContent = `-${formatDuration(maxVal)}`;
        document.getElementById('timeline-current').textContent = secondsAgo > 10 ? `-${formatDuration(secondsAgo)}` : 'LIVE';
        document.getElementById('timeline-end').textContent = 'LIVE';
    }
}

function formatDuration(s) {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const sec = Math.floor(s % 60);
    if (h > 0) return `${h}:${m.toString().padStart(2,'0')}:${sec.toString().padStart(2,'0')}`;
    return `${m}:${sec.toString().padStart(2,'0')}`;
}


// ── Archive ───────────────────────────────────────────────────────────────
async function loadArchiveDates(camId) {
    try {
        const dates = await API.getArchiveDates(camId);
        const input = document.getElementById('archive-date');
        if (dates.length) {
            input.min = dates[0];
            input.max = dates[dates.length - 1];
        }
    } catch { /* no archive data */ }
}

function loadArchive() {
    if (!currentCamera) return;
    const dateStr = document.getElementById('archive-date').value;
    if (!dateStr) return;

    // Enter DVR mode starting from the beginning of the recording
    enterDvrMode(0);
}

// ═══════════════════════════════════════════════════════════════════════════
// AUTH
// ═══════════════════════════════════════════════════════════════════════════
async function handleLogin(e) {
    e.preventDefault();
    const u = document.getElementById('login-username').value;
    const p = document.getElementById('login-password').value;
    const errEl = document.getElementById('login-error');

    try {
        const data = await API.login(u, p);
        API.setAuth(data.token, data.username, data.role);
        closeModals();
        updateAuthUI();
        toast('Logged in successfully');
        loadCameras();
    } catch (err) {
        errEl.textContent = err.message;
        errEl.classList.remove('hidden');
    }
}

function logout() {
    API.clearAuth();
    updateAuthUI();
    showGrid();
    toast('Logged out');
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: CAMERAS
// ═══════════════════════════════════════════════════════════════════════════
function switchAdminTab(tab, el) {
    document.querySelectorAll('.admin-tab-content').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
    document.getElementById(`admin-${tab}`).classList.add('active');
    el.classList.add('active');
}

async function loadAdminCameras() {
    try {
        const cams = await API.getCameras();
        const list = document.getElementById('admin-camera-list');
        if (!cams.length) {
            list.innerHTML = '<p style="color:var(--text-muted);text-align:center;padding:2rem">No cameras yet</p>';
            return;
        }
        list.innerHTML = cams.map(c => `
            <div class="admin-item">
                <div class="admin-item-info">
                    <h4>${escapeHtml(c.name)}</h4>
                    <p>${c.source_type === 'hls' ? 'HLS: ' + (c.stream_url || 'N/A') : 'Stream Key: ' + (c.stream_key || 'N/A')} · Rewind: ${c.rewind_hours}h</p>
                </div>
                <div class="admin-item-actions">
                    ${c.stream_key ? `<button class="btn-ghost" onclick="showStreamKeyInfo('${c.stream_key}')">Key</button>` : ''}
                    <button class="btn-ghost" onclick="editCamera(${c.id})">Edit</button>
                    <button class="btn-danger" onclick="confirmDeleteCamera(${c.id})">Delete</button>
                </div>
            </div>
        `).join('');
    } catch (e) { toast(e.message, 'error'); }
}

function showCameraModal(cam = null) {
    document.getElementById('camera-modal-title').textContent = cam ? 'Edit Camera' : 'Add Camera';
    document.getElementById('camera-edit-id').value = cam ? cam.id : '';
    document.getElementById('camera-name').value = cam ? cam.name : '';
    document.getElementById('camera-location').value = cam ? (cam.location || '') : '';
    document.getElementById('camera-source-type').value = cam ? cam.source_type : 'hls';
    document.getElementById('camera-stream-url').value = cam ? (cam.stream_url || '') : '';
    document.getElementById('camera-rewind').value = cam ? cam.rewind_hours : 24;
    document.getElementById('camera-error').classList.add('hidden');
    toggleSourceFields();
    document.getElementById('modal-camera').classList.remove('hidden');
}

function toggleSourceFields() {
    const type = document.getElementById('camera-source-type').value;
    document.getElementById('field-stream-url').style.display = type === 'hls' ? '' : 'none';
    document.getElementById('field-stream-key-info').classList.toggle('hidden', type !== 'stream_key');
}

async function handleCameraSubmit(e) {
    e.preventDefault();
    const id = document.getElementById('camera-edit-id').value;
    const data = {
        name: document.getElementById('camera-name').value,
        location: document.getElementById('camera-location').value || null,
        source_type: document.getElementById('camera-source-type').value,
        stream_url: document.getElementById('camera-stream-url').value || null,
        rewind_hours: parseInt(document.getElementById('camera-rewind').value) || 24,
    };
    const errEl = document.getElementById('camera-error');

    try {
        if (id) {
            await API.updateCamera(id, data);
            toast('Camera updated');
        } else {
            const result = await API.createCamera(data);
            toast('Camera created');
            if (result.stream_key) {
                closeModals();
                showStreamKeyInfo(result.stream_key);
                loadAdminCameras();
                return;
            }
        }
        closeModals();
        loadAdminCameras();
    } catch (err) {
        errEl.textContent = err.message;
        errEl.classList.remove('hidden');
    }
}

async function editCamera(id) {
    try {
        const cam = await API.getCamera(id);
        showCameraModal(cam);
    } catch (e) { toast(e.message, 'error'); }
}

async function confirmDeleteCamera(id) {
    if (!confirm('Delete this camera and all its recordings?')) return;
    try {
        await API.deleteCamera(id);
        toast('Camera deleted');
        loadAdminCameras();
    } catch (e) { toast(e.message, 'error'); }
}

function showStreamKeyInfo(key) {
    const host = window.location.hostname;
    document.getElementById('rtmp-server-url').value = `rtmp://${host}:1935/live`;
    document.getElementById('rtmp-stream-key').value = key;
    document.getElementById('modal-stream-key').classList.remove('hidden');
}

function copyField(inputId) {
    const input = document.getElementById(inputId);
    navigator.clipboard.writeText(input.value).then(() => toast('Copied!')).catch(() => {
        input.select(); document.execCommand('copy'); toast('Copied!');
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: USERS
// ═══════════════════════════════════════════════════════════════════════════
async function loadAdminUsers() {
    try {
        const users = await API.getUsers();
        const list = document.getElementById('admin-user-list');
        list.innerHTML = users.map(u => `
            <div class="admin-item">
                <div class="admin-item-info">
                    <h4>${escapeHtml(u.username)}</h4>
                    <p>Role: ${u.role} · Created: ${u.created_at}</p>
                </div>
                <div class="admin-item-actions">
                    <button class="btn-ghost" onclick="editUser(${u.id}, '${escapeHtml(u.username)}', '${u.role}')">Edit</button>
                    <button class="btn-danger" onclick="confirmDeleteUser(${u.id})">Delete</button>
                </div>
            </div>
        `).join('');
    } catch (e) { toast(e.message, 'error'); }
}

function showUserModal(user = null) {
    document.getElementById('user-modal-title').textContent = user ? 'Edit User' : 'Add User';
    document.getElementById('user-edit-id').value = user ? user.id : '';
    document.getElementById('user-username').value = user ? user.username : '';
    document.getElementById('user-password').value = '';
    document.getElementById('user-role').value = user ? user.role : 'user';
    document.getElementById('user-username').readOnly = !!user;
    document.getElementById('user-password-hint').classList.toggle('hidden', !user);
    document.getElementById('user-password').required = !user;
    document.getElementById('user-error').classList.add('hidden');
    document.getElementById('modal-user').classList.remove('hidden');
}

async function handleUserSubmit(e) {
    e.preventDefault();
    const id = document.getElementById('user-edit-id').value;
    const errEl = document.getElementById('user-error');

    try {
        if (id) {
            const data = { role: document.getElementById('user-role').value };
            const pw = document.getElementById('user-password').value;
            if (pw) data.password = pw;
            await API.updateUser(id, data);
            toast('User updated');
        } else {
            await API.createUser({
                username: document.getElementById('user-username').value,
                password: document.getElementById('user-password').value,
                role: document.getElementById('user-role').value,
            });
            toast('User created');
        }
        closeModals();
        loadAdminUsers();
    } catch (err) {
        errEl.textContent = err.message;
        errEl.classList.remove('hidden');
    }
}

function editUser(id, username, role) {
    showUserModal({ id, username, role });
}

async function confirmDeleteUser(id) {
    if (!confirm('Delete this user?')) return;
    try {
        await API.deleteUser(id);
        toast('User deleted');
        loadAdminUsers();
    } catch (e) { toast(e.message, 'error'); }
}

// ═══════════════════════════════════════════════════════════════════════════
// ADMIN: SETTINGS
// ═══════════════════════════════════════════════════════════════════════════
async function loadSettings() {
    try {
        const s = await API.getSettings();
        document.getElementById('setting-rewind').value = s.default_rewind_hours || 24;
        document.getElementById('setting-rtmp-port').value = s.rtmp_port || 1935;
    } catch { /* first load, use defaults */ }
}

async function saveSettings() {
    try {
        await API.updateSettings({
            default_rewind_hours: document.getElementById('setting-rewind').value,
            rtmp_port: document.getElementById('setting-rtmp-port').value,
        });
        toast('Settings saved');
    } catch (e) { toast(e.message, 'error'); }
}

// ── Utility ───────────────────────────────────────────────────────────────
function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

// ═══════════════════════════════════════════════════════════════════════════
// MULTI-VIEW
// ═══════════════════════════════════════════════════════════════════════════

function showMvSelector() {
    const list = document.getElementById('mv-camera-list');
    list.innerHTML = cameras.map(cam => {
        const added = mvCameras.some(c => c.id === cam.id);
        return `<div class="admin-item">
            <div class="admin-item-info">
                <h4>${escapeHtml(cam.name)}</h4>
            </div>
            <div class="admin-item-actions">
                ${added 
                    ? `<button class="btn-ghost" disabled>Added</button>` 
                    : `<button class="btn-primary" onclick="addMvCamera(${cam.id})">Add</button>`}
            </div>
        </div>`;
    }).join('');
    document.getElementById('modal-mv-selector').classList.remove('hidden');
}

function saveMvState() {
    const ids = mvCameras.map(c => c.id);
    localStorage.setItem('mvCameras', JSON.stringify(ids));
}

function loadMvState() {
    const saved = localStorage.getItem('mvCameras');
    if (saved) {
        try {
            const ids = JSON.parse(saved);
            mvCameras = ids.map(id => cameras.find(c => c.id === id)).filter(c => c);
        } catch(e) {}
    }
}

function addMvCamera(camId) {
    const cam = cameras.find(c => c.id === camId);
    if (!cam || mvCameras.some(c => c.id === camId)) return;
    
    mvCameras.push(cam);
    saveMvState();
    closeModals();
    renderMultiViewGrid();
}

function removeMvCamera(camId) {
    // Destroy HLS instances for this camera
    const instance = mvHlsInstances[camId];
    if (instance) {
        if (instance.liveHls) instance.liveHls.destroy();
        if (instance.dvrHls) instance.dvrHls.destroy();
        delete mvHlsInstances[camId];
    }
    
    mvCameras = mvCameras.filter(c => c.id !== camId);
    saveMvState();
    
    const container = document.getElementById(`mv-container-${camId}`);
    if (container) container.remove();
    
    if (mvCameras.length === 0) {
        renderMultiViewGrid(); // Shows empty state
    } else {
        refreshMvSliderRange();
    }
}

function renderMultiViewGrid() {
    const grid = document.getElementById('multiview-grid');
    const controls = document.getElementById('mv-controls');
    
    if (mvCameras.length === 0) {
        grid.innerHTML = `<div class="empty-state" id="mv-empty-state">
            <svg width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" opacity="0.3">
                <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
                <line x1="12" y1="3" x2="12" y2="21"/>
                <line x1="3" y1="12" x2="21" y2="12"/>
            </svg>
            <p>No cameras added</p>
            <p class="sub">Click "+ Add Camera" to start multi-view</p>
        </div>`;
        controls.style.display = 'none';
        return;
    }
    
    const emptyState = document.getElementById('mv-empty-state');
    if (emptyState) emptyState.remove();
    controls.style.display = 'block';
    
    mvCameras.forEach(cam => {
        if (!document.getElementById(`mv-container-${cam.id}`)) {
            const div = document.createElement('div');
            div.className = 'multiview-player-container';
            div.id = `mv-container-${cam.id}`;
            div.innerHTML = `
                <div class="multiview-title">${escapeHtml(cam.name)}</div>
                <button class="multiview-remove" onclick="removeMvCamera(${cam.id})">X</button>
                <video id="mv-video-${cam.id}" playsinline muted></video>
                <div id="mv-overlay-${cam.id}" class="player-overlay">
                    <div class="spinner"></div>
                </div>
            `;
            grid.appendChild(div);
            initMvCamera(cam.id);
        }
    });
    
    if (!mvSliderRefreshInterval) {
        refreshMvSliderRange();
        mvSliderRefreshInterval = setInterval(() => {
            if (!mvDvrMode) refreshMvSliderRange();
        }, 10000);
    }
}

function initMvCamera(camId) {
    if (!mvHlsInstances[camId]) {
        mvHlsInstances[camId] = { liveHls: null, dvrHls: null, loaded: false };
    }
    const video = document.getElementById(`mv-video-${camId}`);
    video.addEventListener('timeupdate', onMvVideoTimeUpdate);
    
    if (mvDvrMode) {
        startMvDvrPlayback(camId, video);
    } else {
        startMvLivePlayback(camId, video);
    }
}

function startMvLivePlayback(camId, video) {
    const inst = mvHlsInstances[camId];
    if (inst.dvrHls) { inst.dvrHls.destroy(); inst.dvrHls = null; }
    if (inst.liveHls) { inst.liveHls.destroy(); inst.liveHls = null; }
    
    const url = `/api/cameras/${camId}/live.m3u8`;
    if (Hls.isSupported()) {
        const h = new Hls({ liveSyncDurationCount: 3, enableWorker: true });
        inst.liveHls = h;
        h.loadSource(url);
        h.attachMedia(video);
        h.on(Hls.Events.MANIFEST_PARSED, () => {
            video.play().catch(() => {});
            document.getElementById(`mv-overlay-${camId}`).classList.add('hidden');
        });
        h.on(Hls.Events.ERROR, (_, data) => {
            if (data.fatal && data.type === Hls.ErrorTypes.NETWORK_ERROR) {
                setTimeout(() => h && h.startLoad(), 3000);
            }
        });
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
        video.src = url;
        video.addEventListener('loadedmetadata', () => {
            video.play().catch(() => {});
            document.getElementById(`mv-overlay-${camId}`).classList.add('hidden');
        }, {once: true});
    }
}

async function startMvDvrPlayback(camId, video) {
    const inst = mvHlsInstances[camId];
    if (inst.liveHls) { inst.liveHls.destroy(); inst.liveHls = null; }
    if (inst.dvrHls) { inst.dvrHls.destroy(); inst.dvrHls = null; }
    inst.loaded = false;
    
    document.getElementById(`mv-overlay-${camId}`).classList.remove('hidden');
    
    const url = `/api/cameras/${camId}/full.m3u8`;
    if (Hls.isSupported()) {
        const h = new Hls({
            maxBufferLength: 60,
            maxMaxBufferLength: 120,
            enableWorker: true
        });
        inst.dvrHls = h;
        h.loadSource(url);
        h.attachMedia(video);
        
        h.on(Hls.Events.MANIFEST_PARSED, () => {
            const checkDuration = () => {
                if (video.duration && isFinite(video.duration)) {
                    inst.loaded = true;
                    mvDvrLoadedCount++;
                    const slider = document.getElementById('mv-dvr-slider');
                    const secondsAgo = parseFloat(slider.max) - parseFloat(slider.value);
                    video.currentTime = Math.max(0, video.duration - secondsAgo);
                    video.play().catch(() => {});
                    document.getElementById(`mv-overlay-${camId}`).classList.add('hidden');
                } else {
                    setTimeout(checkDuration, 50);
                }
            };
            checkDuration();
        });
    } else if (video.canPlayType('application/vnd.apple.mpegurl')) {
        video.src = url;
        video.addEventListener('loadedmetadata', () => {
            const checkDuration = () => {
                if (video.duration && isFinite(video.duration)) {
                    inst.loaded = true;
                    mvDvrLoadedCount++;
                    const slider = document.getElementById('mv-dvr-slider');
                    const secondsAgo = parseFloat(slider.max) - parseFloat(slider.value);
                    video.currentTime = Math.max(0, video.duration - secondsAgo);
                    video.play().catch(() => {});
                    document.getElementById(`mv-overlay-${camId}`).classList.add('hidden');
                } else {
                    setTimeout(checkDuration, 50);
                }
            };
            checkDuration();
        }, { once: true });
    }
}

async function refreshMvSliderRange() {
    let maxDur = 60;
    for (const cam of mvCameras) {
        try {
            const range = await API.getSegmentRange(cam.id);
            if (range.duration_seconds > 0) {
                const maxRewind = cam.rewind_hours * 3600;
                const dur = Math.min(range.duration_seconds, maxRewind);
                if (dur > maxDur) maxDur = dur;
            }
        } catch {}
    }
    mvRecordingDuration = maxDur;
    
    if (!mvDvrMode) {
        const slider = document.getElementById('mv-dvr-slider');
        slider.max = mvRecordingDuration;
        slider.value = mvRecordingDuration;
        updateMvTimelineLabels();
    }
}

function onMvVideoTimeUpdate(e) {
    if (!mvDvrMode || mvIsScrubbing) return;
    const firstCam = mvCameras[0];
    if (!firstCam || e.target.id !== `mv-video-${firstCam.id}`) return;
    
    const video = e.target;
    if (!video.duration || !isFinite(video.duration)) return;
    
    const slider = document.getElementById('mv-dvr-slider');
    const secondsAgo = video.duration - video.currentTime;
    slider.value = parseFloat(slider.max) - secondsAgo;
    updateMvTimelineLabels();
}

let mvScrubThrottle = null;

function onMvDvrSliderInput() {
    if (!mvIsScrubbing) {
        // First input event (start of drag): pause all videos
        mvIsScrubbing = true;
        if (mvDvrMode) {
            mvCameras.forEach(cam => {
                const video = document.getElementById(`mv-video-${cam.id}`);
                if (video) video.pause();
            });
        }
    }
    
    updateMvTimelineLabels();
    
    // Throttle visual seeking to 4 times a second to prevent HLS request flooding
    if (mvDvrMode && !mvScrubThrottle) {
        mvScrubThrottle = setTimeout(() => {
            mvScrubThrottle = null;
            if (!mvIsScrubbing) return; // User released slider already
            
            const slider = document.getElementById('mv-dvr-slider');
            const secondsAgo = parseFloat(slider.max) - parseFloat(slider.value);
            mvCameras.forEach(cam => {
                const inst = mvHlsInstances[cam.id];
                if (inst && inst.loaded) {
                    const video = document.getElementById(`mv-video-${cam.id}`);
                    if (video && video.duration) {
                        video.currentTime = Math.max(0, video.duration - secondsAgo);
                    }
                }
            });
        }, 250);
    }
}

function onMvDvrSliderChange() {
    mvIsScrubbing = false;
    const slider = document.getElementById('mv-dvr-slider');
    const secondsAgo = parseFloat(slider.max) - parseFloat(slider.value);
    
    if (secondsAgo < 10) {
        mvJumpToLive();
        return;
    }
    
    if (!mvDvrMode) {
        enterMvDvrMode(secondsAgo);
    } else {
        performMvSync(secondsAgo);
    }
}

let mvSyncReadyCount = 0;

function performMvSync(secondsAgo) {
    mvSyncReadyCount = 0;
    
    const camerasToSync = mvCameras.filter(cam => {
        const inst = mvHlsInstances[cam.id];
        return inst && inst.loaded;
    });
    
    if (camerasToSync.length === 0) return;
    
    camerasToSync.forEach(cam => {
        const video = document.getElementById(`mv-video-${cam.id}`);
        if (!video || !video.duration) return;
        
        document.getElementById(`mv-overlay-${cam.id}`).classList.remove('hidden');
        video.pause();
        
        const checkReady = () => {
            mvSyncReadyCount++;
            if (mvSyncReadyCount >= camerasToSync.length) {
                camerasToSync.forEach(c => {
                    const v = document.getElementById(`mv-video-${c.id}`);
                    document.getElementById(`mv-overlay-${c.id}`).classList.add('hidden');
                    if (v) v.play().catch(()=>{});
                });
            }
        };
        
        const targetTime = Math.max(0, video.duration - secondsAgo);
        
        if (Math.abs(video.currentTime - targetTime) < 0.5 && video.readyState >= 3) {
            checkReady();
            return;
        }
        
        const handler = () => {
            if (video.readyState >= 3) {
                video.removeEventListener('seeked', handler);
                checkReady();
            } else {
                const canplayHandler = () => {
                    video.removeEventListener('canplay', canplayHandler);
                    video.removeEventListener('seeked', handler);
                    checkReady();
                };
                video.addEventListener('canplay', canplayHandler, { once: true });
            }
        };
        
        video.addEventListener('seeked', handler);
        video.currentTime = targetTime;
    });
}
function enterMvDvrMode(initialSecondsAgo) {
    if (mvDvrMode) return;
    
    mvDvrMode = true;
    mvDvrLoadedCount = 0;
    
    document.getElementById('mv-badge-live').classList.add('hidden');
    document.getElementById('mv-badge-dvr').classList.remove('hidden');
    
    const slider = document.getElementById('mv-dvr-slider');
    slider.value = parseFloat(slider.max) - initialSecondsAgo;
    updateMvTimelineLabels();
    
    mvCameras.forEach(cam => {
        const video = document.getElementById(`mv-video-${cam.id}`);
        startMvDvrPlayback(cam.id, video);
    });
}

function mvJumpToLive() {
    mvDvrMode = false;
    document.getElementById('mv-badge-live').classList.remove('hidden');
    document.getElementById('mv-badge-dvr').classList.add('hidden');
    
    const slider = document.getElementById('mv-dvr-slider');
    slider.max = mvRecordingDuration;
    slider.value = mvRecordingDuration;
    updateMvTimelineLabels();
    
    mvCameras.forEach(cam => {
        const video = document.getElementById(`mv-video-${cam.id}`);
        document.getElementById(`mv-overlay-${cam.id}`).classList.remove('hidden');
        startMvLivePlayback(cam.id, video);
    });
}

function mvRewindBy(seconds) {
    const slider = document.getElementById('mv-dvr-slider');
    const currentVal = parseFloat(slider.value);
    slider.value = Math.max(0, currentVal - seconds);
    onMvDvrSliderChange();
}

function updateMvTimelineLabels() {
    const slider = document.getElementById('mv-dvr-slider');
    const maxVal = parseFloat(slider.max) || 1;
    const val = parseFloat(slider.value) || 0;
    
    if (mvDvrMode) {
        document.getElementById('mv-timeline-start').textContent = '0:00';
        document.getElementById('mv-timeline-current').textContent = formatDuration(val);
        document.getElementById('mv-timeline-end').textContent = formatDuration(maxVal);
    } else {
        const secondsAgo = maxVal - val;
        document.getElementById('mv-timeline-start').textContent = `-${formatDuration(maxVal)}`;
        document.getElementById('mv-timeline-current').textContent = secondsAgo > 10 ? `-${formatDuration(secondsAgo)}` : 'LIVE';
        document.getElementById('mv-timeline-end').textContent = 'LIVE';
    }
}

function destroyMultiView() {
    Object.keys(mvHlsInstances).forEach(camId => {
        const inst = mvHlsInstances[camId];
        if (inst.liveHls) inst.liveHls.destroy();
        if (inst.dvrHls) inst.dvrHls.destroy();
        
        const video = document.getElementById(`mv-video-${camId}`);
        if (video) {
            video.pause();
            video.removeAttribute('src');
            video.load();
        }
    });
    
    mvHlsInstances = {};
    mvCameras = [];
    mvDvrMode = false;
    mvRecordingDuration = 0;
    mvIsScrubbing = false;
    if (mvSliderRefreshInterval) {
        clearInterval(mvSliderRefreshInterval);
        mvSliderRefreshInterval = null;
    }
    
    const grid = document.getElementById('multiview-grid');
    if (grid) grid.innerHTML = '';
}
