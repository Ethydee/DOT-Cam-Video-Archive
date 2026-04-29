// ═══════════════════════════════════════════════════════════════════════════
// API Client — handles all communication with the backend
// ═══════════════════════════════════════════════════════════════════════════

const API = {
    token: () => localStorage.getItem('dvr_token'),
    role: () => localStorage.getItem('dvr_role'),
    username: () => localStorage.getItem('dvr_username'),

    setAuth(token, username, role) {
        localStorage.setItem('dvr_token', token);
        localStorage.setItem('dvr_username', username);
        localStorage.setItem('dvr_role', role);
    },

    clearAuth() {
        localStorage.removeItem('dvr_token');
        localStorage.removeItem('dvr_username');
        localStorage.removeItem('dvr_role');
    },

    isLoggedIn() { return !!this.token(); },
    isAdmin() { return this.role() === 'admin'; },

    headers(json = false) {
        const h = {};
        if (this.token()) h['Authorization'] = `Bearer ${this.token()}`;
        if (json) h['Content-Type'] = 'application/json';
        return h;
    },

    async request(method, path, body = null) {
        const opts = { method, headers: this.headers(!!body) };
        if (body) opts.body = JSON.stringify(body);
        const res = await fetch(path, opts);
        const data = await res.json().catch(() => null);
        if (!res.ok) throw new Error(data?.error || `Request failed (${res.status})`);
        return data;
    },

    // Auth
    login: (u, p) => API.request('POST', '/api/auth/login', { username: u, password: p }),
    me: () => API.request('GET', '/api/auth/me'),

    // Cameras (public)
    getCameras: () => API.request('GET', '/api/cameras'),
    getCamera: (id) => API.request('GET', `/api/cameras/${id}`),
    getArchiveDates: (id) => API.request('GET', `/api/cameras/${id}/dates`),
    getSegmentRange: (id) => API.request('GET', `/api/cameras/${id}/range`),

    // Admin: Cameras
    createCamera: (data) => API.request('POST', '/api/admin/cameras', data),
    updateCamera: (id, data) => API.request('PUT', `/api/admin/cameras/${id}`, data),
    deleteCamera: (id) => API.request('DELETE', `/api/admin/cameras/${id}`),

    // Admin: Users
    getUsers: () => API.request('GET', '/api/admin/users'),
    createUser: (data) => API.request('POST', '/api/admin/users', data),
    updateUser: (id, data) => API.request('PUT', `/api/admin/users/${id}`, data),
    deleteUser: (id) => API.request('DELETE', `/api/admin/users/${id}`),

    // Admin: Settings
    getSettings: () => API.request('GET', '/api/admin/settings'),
    updateSettings: (data) => API.request('PUT', '/api/admin/settings', data),
};
