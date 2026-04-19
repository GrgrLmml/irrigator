(function () {
  'use strict';

  // ---- Config ----
  const POLL_OPEN_MS = 2000;
  const POLL_CLOSED_MS = 10000;
  const BACKOFF_MAX_MS = 30000;
  const SPARK_SAMPLES = 60;

  // ---- Elements ----
  const $ = (id) => document.getElementById(id);
  const el = {
    body: document.body,
    banner_schedule_off: $('banner-schedule-off'),
    banner_anomaly: $('banner-anomaly'),
    banner_offline: $('banner-offline'),
    conn_dot: $('conn-dot'),
    clock: $('clock'),
    status_text: $('status-text'),
    source_chip: $('source-chip'),
    droplets: $('droplets'),
    m_elapsed: $('m-elapsed'),
    m_volume: $('m-volume'),
    m_flow: $('m-flow'),
    cd: $('cd'),
    next_slot_text: $('next-slot-text'),
    btn_start: $('btn-start'),
    btn_stop: $('btn-stop'),
    ring_fill: $('ring-fill'),
    ring_value: $('ring-value'),
    ring_target: $('ring-target'),
    today_sessions: $('today-sessions'),
    today_pct: $('today-pct'),
    today_last: $('today-last'),
    btn_target: $('btn-target'),
    spark_now: $('spark-now'),
    spark_peak: $('spark-peak'),
    spark_line: $('spark-line'),
    spark_area: $('spark-area'),
    week: $('week'),
    sched_toggle: $('sched-toggle'),
    slots: $('slots'),
    hist: $('hist'),
    hist_empty: $('hist-empty'),
    stat_total: $('stat-total'),
    stat_avg: $('stat-avg'),
    stat_peak: $('stat-peak'),
    stat_uptime: $('stat-uptime'),
    version: $('version'),
    quick_chips: $('quick-chips'),
  };

  // ---- State ----
  let state = { status: null, summary: null, schedule: null, history: null };
  let sparkBuf = [];
  let backoff = POLL_OPEN_MS;
  let pollTimer = null;
  let summaryTimer = null;

  // ---- Droplets (static DOM, CSS-animated) ----
  for (let i = 0; i < 9; i++) {
    const d = document.createElement('div');
    d.className = 'droplet';
    d.style.left = (6 + i * 11) + '%';
    d.style.animationDelay = (-i * 0.14) + 's';
    el.droplets.appendChild(d);
  }

  // ---- Formatters ----
  const fmtSecs = (s) => {
    if (s == null || s < 0) return '—';
    const m = Math.floor(s / 60);
    const r = s % 60;
    return m + ':' + String(r).padStart(2, '0');
  };
  const fmtLiters = (l) => (l == null ? '—' : (l >= 100 ? l.toFixed(0) : l.toFixed(1)) + ' L');
  const fmtLpm = (v) => (v == null ? '—' : v.toFixed(1));
  const fmtClock = (iso) => {
    const d = new Date(iso);
    return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
  };
  const fmtUptime = (secs) => {
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (h >= 24) return Math.floor(h / 24) + 'd ' + (h % 24) + 'h';
    if (h > 0) return h + 'h ' + m + 'm';
    return m + 'm';
  };

  // ---- Rendering ----
  function renderStatus(st) {
    el.body.classList.toggle('open', st.valve_open);
    el.body.classList.toggle('anomaly', !!st.anomaly);
    el.status_text.textContent = st.valve_open ? 'OPEN' : 'CLOSED';

    if (st.valve_open) {
      const src = st.session_source || '';
      const chip = src === 'schedule' ? 'Schedule' : (src === 'telegram' ? 'Telegram' : (src === 'web' ? 'Manual' : 'Active'));
      el.source_chip.textContent = st.anomaly ? chip + ' · low flow!' : chip;
      el.m_elapsed.textContent = fmtSecs(st.session_seconds);
      el.m_volume.textContent = fmtLiters(st.session_liters);
      el.m_flow.textContent = fmtLpm(st.session_flow_lpm);
      el.cd.textContent = st.auto_off_seconds_remaining != null
        ? fmtSecs(Math.max(0, st.auto_off_seconds_remaining))
        : '—';

      // Sparkline ring buffer
      sparkBuf.push(st.session_flow_lpm);
      if (sparkBuf.length > SPARK_SAMPLES) sparkBuf.shift();
      el.spark_now.textContent = fmtLpm(st.session_flow_lpm);
      el.spark_peak.textContent = fmtLpm(Math.max(...sparkBuf, 0));
      renderSparkline(sparkBuf);
    } else {
      el.source_chip.textContent = '';
      sparkBuf = [];
      if (st.next_slot) {
        const s = st.next_slot;
        const t = String(s.hour).padStart(2, '0') + ':' + String(s.minute).padStart(2, '0');
        el.next_slot_text.textContent = t + ' ' + s.when + ' (' + s.duration_min + ' min)';
      } else {
        el.next_slot_text.textContent = 'no scheduled slots';
      }
    }

    // Banners
    el.banner_schedule_off.classList.toggle('show', !st.schedule_enabled);
    el.banner_anomaly.classList.toggle('show', !!st.anomaly);

    // Stats uptime
    el.stat_uptime.textContent = fmtUptime(st.uptime_seconds);
  }

  function renderSparkline(data) {
    if (data.length < 2) {
      el.spark_line.setAttribute('d', '');
      el.spark_area.setAttribute('d', '');
      return;
    }
    const W = 300, H = 56, pad = 4;
    const max = Math.max(1.5, ...data);
    const n = data.length;
    const pts = data.map((v, i) => {
      const x = (i / Math.max(1, n - 1)) * W;
      const y = H - pad - (v / max) * (H - pad * 2);
      return x.toFixed(1) + ',' + y.toFixed(1);
    });
    const line = 'M' + pts.join(' L');
    const area = line + ` L${W},${H} L0,${H} Z`;
    el.spark_line.setAttribute('d', line);
    el.spark_area.setAttribute('d', area);
  }

  function renderSummary(s) {
    // Today
    const t = s.today;
    const pct = t.target_liters > 0 ? Math.min(1, t.liters / t.target_liters) : 0;
    const circ = 314.16;
    el.ring_fill.setAttribute('stroke-dashoffset', circ * (1 - pct));
    el.ring_value.textContent = fmtLiters(t.liters);
    el.ring_target.textContent = t.target_liters >= 100 ? t.target_liters.toFixed(0) : t.target_liters.toFixed(0);
    el.today_sessions.textContent = t.sessions + ' / ' + t.minutes + ' min';
    el.today_pct.textContent = Math.round(pct * 100) + '%';

    // 7-day bars
    const days = s.last_7_days;
    const max = Math.max(1, ...days.map((d) => d.liters));
    el.week.innerHTML = '';
    const today_iso = new Date().toISOString().slice(0, 10);
    days.forEach((d) => {
      const [, m, dd] = d.date.split('-');
      const isToday = d.date === today_iso;
      const label = isToday ? 'Today' : labelFor(d.date);
      const row = document.createElement('div');
      row.className = 'day-row' + (isToday ? ' today' : '');
      row.innerHTML =
        '<div class="day-label">' + label + '</div>' +
        '<div class="day-bar"><div class="day-bar-fill" style="width:' + ((d.liters / max) * 100).toFixed(1) + '%"></div></div>' +
        '<div class="day-value">' + fmtLiters(d.liters) + '</div>';
      el.week.appendChild(row);
    });

    // Stats
    el.stat_total.textContent = fmtLiters(s.lifetime.liters);
    el.stat_avg.textContent = fmtLiters(s.avg_session_liters);
    el.stat_peak.textContent = fmtLpm(s.peak_flow_lpm) + ' L/min';
  }

  function labelFor(dateIso) {
    const [, , dd] = dateIso.split('-');
    const date = new Date(dateIso + 'T00:00:00');
    const wd = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][date.getDay()];
    return wd;
  }

  function renderSchedule(s) {
    el.body.classList.toggle('sched-on', s.enabled);
    el.sched_toggle.classList.toggle('on', s.enabled);
    el.slots.innerHTML = '';
    s.slots.forEach((slot) => {
      const row = document.createElement('div');
      row.className = 'slot-row';
      const t = String(slot.hour).padStart(2, '0') + ':' + String(slot.minute).padStart(2, '0');
      row.innerHTML =
        '<div><span class="time">' + t + '</span><span class="dur">' + slot.duration_min + ' min</span></div>' +
        '<button class="icon-btn" title="Edit" disabled>✎</button>';
      el.slots.appendChild(row);
    });
  }

  function renderHistory(h) {
    el.hist.innerHTML = '';
    if (!h.events || h.events.length === 0) {
      el.hist_empty.classList.add('show');
      el.today_last.textContent = '—';
      return;
    }
    el.hist_empty.classList.remove('show');
    const todayIso = new Date().toISOString().slice(0, 10);
    let lastToday = null;
    h.events.forEach((e) => {
      const d = new Date(e.timestamp);
      const iso = d.toISOString().slice(0, 10);
      const timeStr = iso === todayIso ? fmtClock(e.timestamp) : labelFor(iso).toLowerCase() + ' ' + fmtClock(e.timestamp);
      if (iso === todayIso && !lastToday) lastToday = fmtClock(e.timestamp);
      const row = document.createElement('div');
      row.className = 'hist-row';
      row.innerHTML =
        '<div class="t">' + timeStr + '</div>' +
        '<div class="d">' + e.duration_min + ' min</div>' +
        '<div class="v">' + fmtLiters(e.volume_liters) + '</div>' +
        '<div class="src ' + e.source + '">' + e.source + '</div>';
      el.hist.appendChild(row);
    });
    el.today_last.textContent = lastToday || '—';
  }

  // ---- Polling ----
  async function fetchJSON(url, opts) {
    const r = await fetch(url, opts || {});
    if (!r.ok) throw new Error(url + ' -> ' + r.status);
    return r.json();
  }

  async function tick() {
    try {
      const st = await fetchJSON('/api/status');
      state.status = st;
      renderStatus(st);
      el.banner_offline.classList.remove('show');
      el.conn_dot.classList.remove('offline');
      backoff = st.valve_open ? POLL_OPEN_MS : POLL_CLOSED_MS;
    } catch (e) {
      el.banner_offline.classList.add('show');
      el.conn_dot.classList.add('offline');
      backoff = Math.min(backoff * 2, BACKOFF_MAX_MS);
    }
    schedule();
  }

  function schedule() {
    clearTimeout(pollTimer);
    if (document.visibilityState === 'hidden') return;
    pollTimer = setTimeout(tick, backoff);
  }

  async function refreshSummary() {
    try {
      const [sum, sch, his] = await Promise.all([
        fetchJSON('/api/summary'),
        fetchJSON('/api/schedule'),
        fetchJSON('/api/history?limit=10'),
      ]);
      state.summary = sum;
      state.schedule = sch;
      state.history = his;
      renderSummary(sum);
      renderSchedule(sch);
      renderHistory(his);
    } catch (e) { /* quiet */ }
  }

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') {
      tick();
      refreshSummary();
    } else {
      clearTimeout(pollTimer);
    }
  });

  // ---- Actions ----
  function vibrate(pattern) { if (navigator.vibrate) navigator.vibrate(pattern); }

  function toast(msg) {
    let t = document.querySelector('.toast');
    if (!t) {
      t = document.createElement('div');
      t.className = 'toast';
      document.body.appendChild(t);
    }
    t.textContent = msg;
    t.classList.add('show');
    clearTimeout(t._h);
    t._h = setTimeout(() => t.classList.remove('show'), 2500);
  }

  async function openValve(minutes) {
    vibrate(30);
    try {
      const st = await fetchJSON('/api/valve/open', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ duration_min: minutes }),
      });
      state.status = st;
      renderStatus(st);
      toast('Opened for ' + minutes + ' min');
      refreshSummary();
    } catch (e) {
      toast('Failed to open valve');
    }
  }

  async function closeValve() {
    vibrate([30, 50, 30]);
    try {
      const st = await fetchJSON('/api/valve/close', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
      });
      state.status = st;
      renderStatus(st);
      toast('Valve closed');
      refreshSummary();
    } catch (e) {
      toast('Failed to close valve');
    }
  }

  el.btn_start.addEventListener('click', () => openValve(10));
  el.btn_stop.addEventListener('click', () => closeValve());
  el.quick_chips.addEventListener('click', (e) => {
    const m = e.target.dataset && e.target.dataset.min;
    if (m) openValve(parseInt(m, 10));
  });

  el.sched_toggle.addEventListener('click', async () => {
    const next = !el.sched_toggle.classList.contains('on');
    vibrate(20);
    try {
      const s = await fetchJSON('/api/schedule/enabled', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ enabled: next }),
      });
      renderSchedule(s);
      toast(next ? 'Schedule enabled' : 'Schedule disabled');
    } catch (e) {
      toast('Failed to toggle schedule');
    }
  });

  el.btn_target.addEventListener('click', async () => {
    const cur = state.summary ? state.summary.daily_target_liters : 40;
    const v = prompt('Daily target (liters):', cur);
    if (v == null) return;
    const n = parseFloat(v);
    if (isNaN(n) || n < 0) { toast('Invalid number'); return; }
    try {
      await fetchJSON('/api/settings/daily_target', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ liters: n }),
      });
      toast('Target updated');
      refreshSummary();
    } catch (e) {
      toast('Failed to save target');
    }
  });

  // ---- Clock (local time from server TZ offset) ----
  function updateClock() {
    const d = new Date();
    el.clock.textContent = String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
  }
  updateClock();
  setInterval(updateClock, 30000);

  // ---- Service worker ----
  if ('serviceWorker' in navigator) {
    navigator.serviceWorker.register('/sw.js').catch(() => {});
  }

  // ---- Boot ----
  tick();
  refreshSummary();
  summaryTimer = setInterval(refreshSummary, 60000);
})();
