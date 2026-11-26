// Keeps the server status block current without a reload (docs/portal-status.md). It polls
// /api/v1/status, which the portal serves from its cache, every refresh interval while the tab
// is visible, and rebuilds the block from the JSON with textContent only: nothing from the
// answer is ever parsed as HTML. Without this script the server-rendered block stays as it is.
(function () {
  "use strict";
  var root = document.getElementById("status-live");
  if (!root || !window.fetch) return;
  var url = root.getAttribute("data-url") || "/api/v1/status";
  var every = Math.max(5, parseInt(root.getAttribute("data-refresh"), 10) || 15) * 1000;
  var updated = document.getElementById("status-updated");
  var lastOk = Date.parse(root.getAttribute("data-generated")) || Date.now();
  var stale = false;
  var timer = null;
  var busy = false;

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined && text !== null) e.textContent = String(text);
    return e;
  }
  function add(parent, child) {
    parent.appendChild(child);
    return child;
  }
  function fact(dl, term, value) {
    add(dl, el("dt", null, term));
    add(dl, el("dd", null, value));
  }
  function known(value) {
    return value === undefined || value === null ? "unknown" : value;
  }
  function clear(node) {
    while (node.firstChild) node.removeChild(node.firstChild);
  }
  // Only the four classes the stylesheet knows.
  function stateClass(state) {
    return ["up", "degraded", "down"].indexOf(state) >= 0 ? state : "unknown";
  }

  function banner(b) {
    var box = document.getElementById("status-banner");
    if (!box) return;
    clear(box);
    if (!b) return;
    var p = add(box, el("p", "banner"));
    p.setAttribute("role", "status");
    add(p, el("strong", null, "Notice:"));
    add(p, document.createTextNode(" " + b.message));
    if (b.restart_text) {
      add(p, document.createTextNode(" "));
      add(p, el("span", "quiet", "Back by " + b.restart_text + "."));
    }
  }

  function world(w) {
    var li = el("li", w.open ? "world" : "world closed");
    var head = add(li, el("div", "status-head"));
    add(head, el("h4", null, w.modes && w.modes.length ? w.modes.join(" / ") : w.id));
    add(head, el("span", "pill state-" + (w.open ? "up" : "down"), w.state_text));
    var m = w.match;
    if (m) {
      var sides = add(li, el("p"));
      add(sides, el("strong", null, m.attackers));
      add(sides, document.createTextNode(" attacking "));
      add(sides, el("strong", null, m.defenders));
      var dl = add(li, el("dl", "status-facts"));
      fact(dl, "Players", m.humans_text);
      fact(dl, "Sides", m.attacking_humans + " humans attacking, " + m.defending_humans + " defending");
      fact(dl, "Bots", m.bots);
      add(dl, el("dt", null, "Map profile"));
      add(add(dl, el("dd")), el("code", null, m.profile));
    } else {
      add(li, el("p", "quiet", "No match in progress. " + w.humans_text));
    }
    add(li, el("p", "quiet", "Waiting: " + w.queued));
    return li;
  }

  function server(s) {
    var section = el("section", "status-server");
    section.id = "server-" + s.id;
    var head = add(section, el("div", "status-head"));
    var names = add(head, el("div"));
    add(names, el("h3", null, s.name));
    if (s.host) add(add(names, el("p", "quiet")), el("code", null, s.host));
    add(head, el("span", "pill state-" + stateClass(s.state), s.state_label));
    var worlds = s.worlds || [];
    var summary = s.summary;
    if (!worlds.length && s.last_seen_text) summary += " Last seen " + s.last_seen_text + ".";
    add(section, el("p", null, summary));
    if (!worlds.length) return section;
    var dl = add(section, el("dl", "status-facts"));
    if (s.players_text) fact(dl, "Players online", s.players_text);
    fact(dl, "Waiting for a match", known(s.queued));
    if (s.dme_ports) {
      fact(dl, "Game ports in use", s.dme_ports.in_use +
        (s.dme_ports.capacity === null || s.dme_ports.capacity === undefined ? "" : " of " + s.dme_ports.capacity));
    }
    if (s.uptime_text) fact(dl, "Up for", s.uptime_text);
    fact(dl, "Versions", "Medius " + known(s.medius_version) + " · gameplay " + known(s.gameplay_version));
    var list = add(section, el("ul", "worlds"));
    worlds.forEach(function (w) { add(list, world(w)); });
    return section;
  }

  function render(view) {
    var headline = document.getElementById("status-headline");
    if (headline && view.headline) headline.textContent = view.headline;
    banner(view.banner);
    var servers = document.getElementById("status-servers");
    if (servers) {
      clear(servers);
      (view.servers || []).forEach(function (s) { add(servers, server(s)); });
    }
  }

  function age() {
    if (!updated) return;
    var seconds = Math.max(0, Math.round((Date.now() - lastOk) / 1000));
    var text = seconds < 5 ? "just now" : seconds < 120 ? seconds + " s ago" : Math.round(seconds / 60) + " min ago";
    updated.textContent = stale ? "Could not refresh: showing data from " + text + "." : "Updated " + text + ".";
    updated.classList.toggle("stale", stale);
    root.classList.toggle("stale", stale);
  }

  function schedule() {
    clearTimeout(timer);
    timer = document.hidden ? null : setTimeout(poll, every);
  }

  function poll() {
    if (busy) return;
    busy = true;
    fetch(url, { cache: "no-store", credentials: "omit", headers: { Accept: "application/json" } })
      .then(function (r) {
        if (!r.ok) throw new Error("HTTP " + r.status);
        return r.json();
      })
      .then(function (view) {
        render(view);
        lastOk = Date.now();
        stale = false;
      })
      .catch(function () {
        stale = true;
      })
      .then(function () {
        busy = false;
        age();
        schedule();
      });
  }

  document.addEventListener("visibilitychange", function () {
    if (document.hidden) {
      clearTimeout(timer);
      timer = null;
    } else {
      poll();
    }
  });
  setInterval(function () { if (!document.hidden) age(); }, 1000);
  age();
  schedule();
})();
